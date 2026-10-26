"""Check metadata cleanup independently of XLSX field expectations."""

import importlib.util
import io
import unittest
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET

spec = importlib.util.spec_from_file_location("sanitizer", Path(__file__).with_name("sanitize-metadata.py"))
sanitizer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sanitizer)


def fixture(extra_private=False):
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        archive.writestr("docProps/core.xml", '<cp:coreProperties xmlns:cp="' + sanitizer.CP + '" xmlns:dc="' + sanitizer.DC + '"><dc:creator>Fixture identity</dc:creator><cp:lastModifiedBy>Fixture identity</cp:lastModifiedBy></cp:coreProperties>', compress_type=zipfile.ZIP_DEFLATED)
        archive.writestr("xl/workbook.xml", '<workbook xmlns:x15ac="urn:test"><x15ac:absPath url="file:///Users/fixture/"/><sheet name="Synthetic"/></workbook>')
        archive.writestr("docProps/app.xml", '<Properties><Company>Fixture company</Company><Manager>Fixture manager</Manager></Properties>')
        archive.writestr("xl/worksheets/sheet1.xml", '<worksheet><c r="C2"><f>B2*2</f><v>10</v></c></worksheet>' + ('Fixture identity' if extra_private else ''), compress_type=zipfile.ZIP_DEFLATED)
    return buffer.getvalue()


class SanitizationTests(unittest.TestCase):
    def test_metadata_and_package_integrity(self):
        original = fixture()
        result, report = sanitizer.sanitize(original)
        self.assertNotEqual(original, result)
        self.assertEqual(report["field_counts"], {"existing_identity_fields": 2, "added_identity_fields": 0, "company_manager_fields": 2, "absolute_path_elements": 1})
        self.assertEqual(report["remaining_private_locations"], 0)
        with zipfile.ZipFile(io.BytesIO(original)) as before, zipfile.ZipFile(io.BytesIO(result)) as after:
            self.assertEqual([(i.filename, i.compress_type) for i in before.infolist()], [(i.filename, i.compress_type) for i in after.infolist()])
            self.assertIsNone(after.testzip())
            self.assertEqual(before.read("xl/worksheets/sheet1.xml"), after.read("xl/worksheets/sheet1.xml"))
            core = ET.fromstring(after.read("docProps/core.xml"))
            self.assertEqual(core.find(f"{{{sanitizer.DC}}}creator").text, sanitizer.SYNTHETIC)
            self.assertEqual(core.find(f"{{{sanitizer.CP}}}lastModifiedBy").text, sanitizer.SYNTHETIC)
            self.assertNotIn(b"absPath", after.read("xl/workbook.xml"))
            self.assertTrue(all(e.text is None for e in ET.fromstring(after.read("docProps/app.xml"))))

    def test_private_content_elsewhere_refuses_output(self):
        with self.assertRaisesRegex(ValueError, "remaining private package locations"):
            sanitizer.sanitize(fixture(extra_private=True))

    def test_archive_comment_private_marker_refuses_output(self):
        buffer = io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(fixture())) as source, zipfile.ZipFile(buffer, "w") as target:
            for info in source.infolist():
                target.writestr(info, source.read(info))
            target.comment = b"Fixture identity"
        with self.assertRaisesRegex(ValueError, "remaining private package locations"):
            sanitizer.sanitize(buffer.getvalue())

    def test_missing_creator_is_added(self):
        original = fixture()
        buffer = io.BytesIO()
        with zipfile.ZipFile(io.BytesIO(original)) as source, zipfile.ZipFile(buffer, "w") as target:
            for info in source.infolist():
                data = source.read(info).replace(b'<dc:creator>Fixture identity</dc:creator>', b'')
                target.writestr(info, data)
        _, report = sanitizer.sanitize(buffer.getvalue())
        self.assertEqual(report["field_counts"]["added_identity_fields"], 1)


if __name__ == "__main__":
    unittest.main()
