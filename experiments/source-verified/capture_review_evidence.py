"""Run offline review regressions and capture only their synthetic lab artifacts."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import plistlib
import platform
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
import zipfile

import writer


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def collect_run(run, output):
    if not str(run).startswith('/private/tmp/traeva-c-verified-'):
        raise ValueError('synthetic_run_required')
    output.mkdir(parents=True, exist_ok=True)
    objects = []
    for folder in ('app-data', 'app-data/staging', 'delivery'):
        parent = run / folder
        if not parent.is_dir(): continue
        for item in sorted(parent.iterdir()):
            if item.is_symlink() or not item.is_file(): continue
            fd = os.open(item, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            try:
                before = writer.base.snapshot(fd)
                if before['size'] > writer.MAX_FILE: raise ValueError('artifact_too_large')
                body = os.pread(fd, before['size'] + 1, 0)
                if writer.base.snapshot(fd) != before: raise ValueError('artifact_unstable')
                target = output / folder / item.name
                target.parent.mkdir(parents=True, exist_ok=True); target.write_bytes(body)
                entry = {'file': str(item.relative_to(run)), 'object': before, 'sha256': hashlib.sha256(body).hexdigest()}
                if folder == 'delivery':
                    for name in ('com.apple.quarantine', 'com.apple.metadata:kMDItemWhereFroms'):
                        try:
                            value = writer.attribute(fd, name)
                            entry[name] = plistlib.loads(value) if name.endswith('WhereFroms') else value.decode()
                        except OSError: entry[name] = None
                objects.append(entry)
            finally: os.close(fd)
    (output / 'file-observations.json').write_text(json.dumps(objects, indent=2))
    (output / 'restart-inspection.json').write_text(json.dumps(writer.inspect_restart(run), indent=2))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', required=True)
    parser.add_argument('--chrome', required=True)
    parser.add_argument('--direct', required=True)
    parser.add_argument('--before', required=True)
    parser.add_argument('--r-evidence', required=True)
    args = parser.parse_args()
    output, source = Path(args.output), Path(__file__).resolve().parent
    output.mkdir(parents=True, exist_ok=False)
    runs = []
    temp_roots = []
    make_temp = tempfile.mkdtemp
    def track_temp(*a, **kw):
        value = make_temp(*a, **kw)
        if value.startswith('/private/tmp/traeva-c-verified-'): temp_roots.append(Path(value))
        return value
    tempfile.mkdtemp = track_temp
    original = writer.Writer.__init__
    def track(lab, *a, **kw):
        original(lab, *a, **kw); runs.append(lab.run)
    writer.Writer.__init__ = track
    suite = unittest.defaultTestLoader.discover(str(source), pattern='test_*.py')
    log = io.StringIO()
    result = unittest.TextTestRunner(stream=log, verbosity=2).run(suite)
    writer.Writer.__init__ = original
    tempfile.mkdtemp = make_temp
    (output / 'python-tests.txt').write_text(log.getvalue())
    (output / 'python-results.json').write_text(json.dumps({'testsRun': result.testsRun, 'failures': len(result.failures),
        'errors': len(result.errors), 'result': 'PASS_SELF_TEST' if result.wasSuccessful() else 'FAIL'}, indent=2))
    if not result.wasSuccessful(): raise RuntimeError('regression_failed')
    for root in temp_roots:
        if (root / 'app-data').exists(): runs.append(root)
        elif (root / 'run/app-data').exists(): runs.append(root / 'run')
    for index, run in enumerate(dict.fromkeys(runs)): collect_run(run, output / 'python-runs' / str(index))
    for kind, root in (('chrome', Path(args.chrome)), ('direct', Path(args.direct))):
        shutil.copytree(root / 'evidence', output / kind)
        collect_run(root / 'native', output / kind / 'native-run')
    before = Path(args.before)
    for name in ('before-writer.json', 'before-worker.json', 'capture_before.py', 'after-worker.json', 'chrome-verification.json'):
        shutil.copy2(before / name, output / name)
    shutil.copytree(before / 'baseline', output / 'original-source')
    shutil.copy2(args.r_evidence, output / 'r-original-evidence.zip')
    code = output / 'source'; code.mkdir()
    for file in source.iterdir():
        if file.suffix in ('.py', '.js', '.cjs', '.md', '.txt', '.html'):
            shutil.copy2(file, code / file.name)
    # Input fixtures/source and immutable original FAIL evidence intentionally define
    # synthetic sentinels. Only fixed-run output must not retain their secret values.
    markers = (b'SYNTHETIC_QUERY_SECRET', b'SYNTHETIC_RESPONSE_COOKIE_SECRET', b'SYNTHETIC_PRIVATE_COOKIE',
               b'SYNTHETIC_AUTH_SECRET', b'SYNTHETIC_POST_SECRET', b'SYNTHETIC_ERROR_SECRET')
    count = 0
    for folder in ('chrome', 'direct', 'python-runs'):
        for file in (output / folder).rglob('*'):
            if file.suffix in ('.json', '.jsonl'):
                if any(marker in file.read_bytes() for marker in markers): raise ValueError('private_runtime_artifact')
                count += 1
    (output / 'privacy-artifacts.json').write_text(json.dumps({'runtimeArtifactsChecked': count, 'forbiddenValuesFound': 0,
        'excluded': 'fixture source/input definitions and immutable original FAIL evidence', 'result': 'PASS_SYNTHETIC_PRIVACY'}, indent=2))
    (output / 'environment.json').write_text(json.dumps({'recordedAtNs': time.time_ns(), 'platform': platform.platform(),
        'python': sys.version, 'codeRevision': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=source, text=True).strip(),
        'productionAcceptance': False, 'notes': ['self tests, not R1 independent recheck', 'process exits are not power loss',
        'in-memory syntax check passed; py_compile external Apple cache write was sandbox-refused; no permission bypass']}, indent=2))
    manifest = {str(p.relative_to(output)): {'bytes': p.stat().st_size, 'sha256': sha(p)} for p in sorted(output.rglob('*')) if p.is_file()}
    (output / 'MANIFEST.json').write_text(json.dumps(manifest, indent=2))
    archive = source / 'review-fixes-evidence.zip'
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED) as z:
        for p in sorted(output.rglob('*')):
            if p.is_file(): z.write(p, str(p.relative_to(output)))
    (source / 'review-fixes-sha256.txt').write_text(sha(archive) + '  review-fixes-evidence.zip\n')
    print(json.dumps({'archive': str(archive), 'sha256': sha(archive), 'entries': len(manifest) + 1, 'pythonTests': result.testsRun,
                      'runtimePrivacyChecks': count, 'productionAcceptance': False}))


if __name__ == '__main__': main()
