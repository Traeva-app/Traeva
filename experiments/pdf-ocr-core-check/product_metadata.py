#!/usr/bin/env python3
"""Run the experimental child with a copied product Info.plist, never the product app."""
import argparse
import json
import plistlib
import shutil
import subprocess
from datetime import datetime, timezone
from pathlib import Path

from check import HERE, clean, expected_wire, page_sections_match, sha, strict_wire


def preparation_passes(result):
    return (isinstance(result, dict) and result.get('outcome') == 'finished'
            and result.get('child_reaped') is True and result.get('exit_code') == 0
            and result.get('answer') == {'status': 'ready'})


def wire_passes(result, expected_status, expected_text, expected_pages):
    answer = result.get('answer')
    return (result.get('outcome') == 'finished' and result.get('child_reaped') is True
            and result.get('exit_code') == 0 and strict_wire(answer)
            and answer['status'] == expected_status and answer['text'] == expected_text
            and (page_sections_match(answer['text'], expected_pages)
                 if expected_status == 'partial' else answer['text'] == ''))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plist', type=Path, required=True)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--driver', type=Path, required=True)
    parser.add_argument('--source-sha', required=True)
    args = parser.parse_args()
    driver, source = args.driver.resolve(), args.source.resolve()
    data = args.plist.read_bytes()
    info = plistlib.loads(data)
    assert info['CFBundleIdentifier'] == 'io.github.p30g53.traeva'
    assert info['CFBundleName'] == 'Traeva'
    assert all(t['CFBundleTypeRole'] == 'Viewer' and t['LSHandlerRank'] == 'None'
               for t in info['CFBundleDocumentTypes'])
    truth = json.loads((source/'samples/truth.json').read_text())['samples']
    files = sorted((source/'samples').glob('*.pdf'))
    assert len(truth) == 15 and set(truth) == {p.name for p in files}
    assert all(sha(p) == truth[p.name]['sha256'] for p in files)
    baseline = json.loads((HERE/'evidence/20261013T072424216549Z.json').read_text())
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    app = HERE/'target/bundles'/('ProductMetadata-'+stamp+'.app')
    binary = app/'Contents/MacOS'/info['CFBundleExecutable']
    binary.parent.mkdir(parents=True)
    shutil.copy2(driver, binary)
    (app/'Contents/Info.plist').write_bytes(data)
    subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', '--options', 'runtime', str(app)],
                   check=True, capture_output=True)
    subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)],
                   check=True, capture_output=True)
    signature = subprocess.check_output(['/usr/bin/codesign', '-dvv', str(app)], stderr=subprocess.STDOUT).decode()
    output = HERE/'evidence'/('product-metadata-'+stamp+'.json')
    record = dict(source_sha=args.source_sha, kind='developer metadata reproduction; not product app acceptance',
                  metadata=info, plist_sha256=sha(args.plist), binary_sha256=sha(binary),
                  driver_sha256=sha(driver), script_sha256=sha(__file__), signature=signature,
                  cases={}, preparation=[], complete=False)

    def save():
        output.write_text(json.dumps(clean(record), ensure_ascii=False, indent=2)+'\n')

    def run(mode, content=b'', budget=15000):
        result = subprocess.run([str(driver), '--monitor', str(binary), mode, str(budget), '384'],
                                input=content, capture_output=True, env={}, timeout=budget/1000+8)
        result.check_returncode()
        return json.loads(result.stdout)

    def prepare():
        result = run('--prepare', budget=90000)
        result['verdict'] = 'PASS' if preparation_passes(result) else 'FAIL'
        record['preparation'].append(result)
        save()

    save()
    try:
        prepare()
        for path in files:
            result = run('--recognize', path.read_bytes())
            expected_status, expected_text, expected_pages = expected_wire(baseline['cases'][path.name]['original'])
            valid = wire_passes(result, expected_status, expected_text, expected_pages)
            record['cases'][path.name] = dict(input_sha256=sha(path), result=result,
                verdict='PASS' if valid else 'FAIL', expected_status=expected_status,
                expected_text=expected_text)
            save()
            print(path.name, record['cases'][path.name]['verdict'], flush=True)
        prepare()
        record['complete'] = True
    except BaseException as error:
        record['error'] = repr(error)
        raise
    finally:
        save()
        print(output, flush=True)
    if (any(c['verdict'] != 'PASS' for c in record['cases'].values())
            or len(record['preparation']) != 2
            or any(p['verdict'] != 'PASS' for p in record['preparation'])):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
