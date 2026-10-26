#!/usr/bin/env python3
"""Compare source and port under identical bundle metadata; retain differences."""
import argparse
import json
import plistlib
import subprocess
from datetime import datetime, timezone
from pathlib import Path

from check import HERE, clean, sha


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--old', type=Path, required=True)
    parser.add_argument('--new', type=Path, required=True)
    parser.add_argument('--driver', type=Path, required=True)
    parser.add_argument('--source-sha', required=True)
    args = parser.parse_args()
    source, old, new, driver = [p.resolve() for p in (args.source, args.old, args.new, args.driver)]
    old_plist = old.parent.parent/'Info.plist'
    new_plist = new.parent.parent/'Info.plist'
    assert old_plist.read_bytes() == new_plist.read_bytes(), 'Bundle metadata must match exactly'
    info = plistlib.loads(old_plist.read_bytes())
    for app in (old.parent.parent.parent, new.parent.parent.parent):
        subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)], check=True)
    truth = json.loads((source/'samples/truth.json').read_text())['samples']
    files = sorted((source/'samples').glob('*.pdf'))
    assert len(truth) == 15 and set(truth) == {p.name for p in files}
    assert all(sha(p) == truth[p.name]['sha256'] for p in files)
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    output = HERE/'evidence'/('same-bundle-'+stamp+'.json')
    record = dict(source_sha=args.source_sha, kind='developer migration parity, not OCR accuracy acceptance',
        bundle_identifier=info['CFBundleIdentifier'], plist_sha256=sha(old_plist),
        old_binary_sha256=sha(old), new_binary_sha256=sha(new), driver_sha256=sha(driver),
        script_sha256=sha(__file__), cases={}, complete=False)

    def save():
        output.write_text(json.dumps(clean(record), ensure_ascii=False, indent=2)+'\n')

    def comparable(value):
        return {**{k:value[k] for k in ('status','text','left_out')},
                'pages': [{k:p[k] for k in ('page','source','chars')} for p in value['pages']]}

    save()
    try:
        for path in files:
            previous = subprocess.run([str(old),'--isolated','--auto-language','--max-pages','3',
                '--timeout','15','--memory-mib','384',str(path)],capture_output=True,env={},timeout=23)
            current = subprocess.run([str(driver),'--monitor',str(new),'--inspect','15000','384'],
                input=path.read_bytes(),capture_output=True,env={},timeout=23)
            before = json.loads(previous.stdout)
            after = json.loads(current.stdout)
            equal = (previous.returncode == current.returncode == 0 and after['outcome'] == 'finished'
                     and after['child_reaped'] and comparable(before) == comparable(after['answer']))
            record['cases'][path.name] = dict(input_sha256=sha(path),before=before,after=after,
                                              verdict='PASS' if equal else 'FAIL')
            save()
            print(path.name, record['cases'][path.name]['verdict'],flush=True)
        record['complete'] = True
    except BaseException as error:
        record['error'] = repr(error)
        raise
    finally:
        save()
        print(output,flush=True)
    if any(case['verdict'] != 'PASS' for case in record['cases'].values()):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
