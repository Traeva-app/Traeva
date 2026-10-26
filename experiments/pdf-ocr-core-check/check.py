#!/usr/bin/env python3
"""Local signed-bundle migration checks. Uses only frozen synthetic PDFs."""
import argparse
import hashlib
import json
import os
import platform
import plistlib
import re
import shutil
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def clean(value):
    if isinstance(value, str):
        return value.replace(str(ROOT), '<worktree>').replace(str(Path.home()), '<HOME>')
    if isinstance(value, list):
        return [clean(item) for item in value]
    if isinstance(value, dict):
        return {key: clean(item) for key, item in value.items()}
    return value


def expected_wire(reading):
    pages = reading.get('pages', [])
    if any(page['source'] == 'text_layer' for page in pages):
        return 'unsupported', '', []
    if (any(page['source'] == 'failed' for page in pages)
            or reading.get('reader', {}).get('status') == 'failed'):
        return 'failed', '', []
    recognized = [page for page in pages if page['source'] == 'ocr']
    if not recognized:
        return 'unsupported', '', []
    text = '\n'.join('[Page '+str(page['page'])+']\n'
                     +'\n'.join(line['text'] for line in page['lines']) for page in recognized)
    return 'partial', text, [page['page'] for page in recognized]


def strict_wire(answer):
    return (isinstance(answer, dict) and set(answer) == {'status', 'text', 'note'}
            and isinstance(answer['status'], str)
            and answer['status'] in {'partial', 'unsupported', 'failed'}
            and isinstance(answer['text'], str)
            and (answer['note'] is None or isinstance(answer['note'], str)))


def page_sections_match(text, expected_pages):
    headers = list(re.finditer(r'^\[Page ([1-9][0-9]*)\]$', text, re.MULTILINE))
    if not 1 <= len(headers) <= 3 or headers[0].start() != 0:
        return False
    if [int(header.group(1)) for header in headers] != expected_pages:
        return False
    return all(text[header.end():headers[index+1].start() if index+1 < len(headers)
                    else len(text)].strip() for index, header in enumerate(headers))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--source', required=True, help='frozen #215 experiment directory')
    parser.add_argument('--source-sha', required=True)
    args = parser.parse_args()
    source = Path(args.source).resolve()
    old = source / 'target/release/pdf-ocr-probe'
    driver = HERE / 'target/release/traeva-pdf-ocr-core-check'
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    output = HERE / 'evidence' / (stamp + '.json')
    output.parent.mkdir(exist_ok=True)
    record = dict(source_sha=args.source_sha, kind='developer migration check; not R4',
                  platform=platform.platform(), machine=platform.machine(), started_utc=stamp,
                  protocol_sha256=sha(HERE/'PROTOCOL.md'), binary_sha256=sha(driver),
                  original_binary_sha256=sha(old), checks=[], cases={}, lifecycle=[], signatures=[],
                  source_files={str(p.relative_to(ROOT)): sha(p) for p in
                                sorted((ROOT/'crates/traeva-core/src/pdf_ocr').glob('*.rs'))})

    def save():
        output.write_text(json.dumps(clean(record), ensure_ascii=False, indent=2)+'\n')

    def check(label, condition):
        record['checks'].append(dict(label=label, verdict='PASS' if condition else 'FAIL'))
        save()

    def bundle(label):
        app = HERE / 'target/bundles' / stamp / (label+'.app')
        macos = app/'Contents/MacOS'
        macos.mkdir(parents=True)
        executable = macos/'ocr-core-check'
        shutil.copy2(driver, executable)
        (app/'Contents/Info.plist').write_bytes(plistlib.dumps(dict(
            CFBundleExecutable='ocr-core-check', CFBundleIdentifier='dev.traeva.ocr.port.'+label+'.'+stamp,
            CFBundleName=label, CFBundlePackageType='APPL', CFBundleVersion='1',
            LSMinimumSystemVersion='13.0')))
        subprocess.run(['/usr/bin/codesign','--force','--sign','-','--options','runtime',str(app)],
                       check=True, capture_output=True)
        verify = subprocess.run(['/usr/bin/codesign','--verify','--deep','--strict',str(app)], capture_output=True)
        signature = subprocess.run(['/usr/bin/codesign','-dvv',str(app)], capture_output=True)
        libraries = subprocess.run(['/usr/bin/otool','-L',str(executable)], capture_output=True)
        record['signatures'].append(dict(label=label, verified=verify.returncode == 0,
            signature=signature.stderr.decode(), libraries=libraries.stdout.decode(), sha256=sha(executable)))
        check(label+' strict signature', verify.returncode == 0)
        check(label+' hardened runtime', 'runtime' in signature.stderr.decode())
        linked = [line.strip().split(' (', 1)[0] for line in libraries.stdout.decode().splitlines()[1:]]
        check(label+' system libraries', libraries.returncode == 0 and bool(linked)
              and all(name.startswith(('/System/Library/', '/usr/lib/')) for name in linked))
        return executable

    def run(executable, mode, data=b'', timeout=15000, cap=384, cancel=None):
        command = [str(driver),'--monitor',str(executable),mode,str(timeout),str(cap)]
        if cancel is not None:
            command.append(str(cancel))
        result = subprocess.run(command, input=data, capture_output=True, env={}, timeout=timeout/1000+8)
        if result.returncode:
            raise RuntimeError('monitor failed: '+result.stderr.decode(errors='replace'))
        value = json.loads(result.stdout)
        value['mode'] = mode
        value['budget_ms'] = timeout
        value['memory_mib'] = cap
        return value

    def successful(value):
        return value['outcome'] == 'finished' and value['child_reaped'] and value['exit_code'] == 0

    def preparation(executable, label):
        value = run(executable, '--prepare', timeout=90000)
        value['label'] = label
        record['lifecycle'].append(value)
        check(label, successful(value) and value['answer'] == {'status':'ready'})
        return value

    def comparable(reading):
        return dict(status=reading['status'], text=reading['text'], left_out=reading['left_out'],
                    pages=[dict(page=p['page'],source=p['source'],chars=p['chars']) for p in reading['pages']])

    save()
    try:
        observe = (ROOT/'crates/traeva-core/src/observe.rs').read_text()
        check('private input constant matches product', 'pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;' in observe)
        truth = json.loads((source/'samples/truth.json').read_text())['samples']
        record['truth_sha256'] = sha(source/'samples/truth.json')
        expected_names = set(truth)
        actual_names = {path.name for path in (source/'samples').glob('*.pdf')}
        valid_set = len(expected_names) == 15 and actual_names == expected_names
        check('complete frozen 15-PDF set', valid_set)
        if not valid_set:
            raise ValueError('Frozen PDF set missing or unexpected files')
        for path in sorted((source/'samples').glob('*.pdf')):
            valid_hash = sha(path) == truth[path.name]['sha256']
            check('frozen hash '+path.name, valid_hash)
            if not valid_hash:
                raise ValueError('Frozen sample hash mismatch: '+path.name)
        app = bundle('First')
        preparation(app, 'first new bundle on existing Mac')
        old_prepare = subprocess.run([str(old),'--prepare','--auto-language'], capture_output=True, env={}, timeout=100)
        record['original_preparation'] = dict(exit_code=old_prepare.returncode,
            stdout=old_prepare.stdout.decode(),stderr=old_prepare.stderr.decode())
        save()
        for path in sorted((source/'samples').glob('*.pdf')):
            data = path.read_bytes()
            baseline = subprocess.run([str(old),'--isolated','--auto-language','--max-pages','3',
                '--timeout','15','--memory-mib','384',str(path)],capture_output=True,env={},timeout=23)
            old_read = json.loads(baseline.stdout)
            detailed = run(app,'--inspect',data)
            wire = run(app,'--recognize',data)
            record['cases'][path.name] = dict(input_sha256=sha(path), input_bytes=len(data),
                original=old_read,detailed=detailed,wire=wire)
            check(path.name+' detailed parity', successful(detailed) and comparable(old_read) == comparable(detailed['answer']))
            answer = wire['answer']
            check(path.name+' strict wire', successful(wire) and strict_wire(answer))
            expected, expected_text, expected_pages = expected_wire(old_read)
            check(path.name+' result mapping', strict_wire(answer) and answer['status'] == expected
                  and answer['text'] == expected_text)
            check(path.name+' wire page sections', strict_wire(answer)
                  and (page_sections_match(answer['text'], expected_pages) if expected == 'partial'
                       else answer['text'] == ''))
            print(path.name, record['checks'][-4:], flush=True)
        for i in range(2):
            preparation(app,'subsequent process '+str(i+1))
        for label, kwargs in [('cancel',dict(cancel=1)),('deadline',dict(timeout=1)),
                               ('memory',dict(cap=64))]:
            value = run(app,'--recognize',(source/'samples/s13-long-scan.pdf').read_bytes(),**kwargs)
            value['label'] = label
            record['lifecycle'].append(value)
            expected = {'cancel':'cancelled','deadline':'time_limit','memory':'memory_limit'}[label]
            check(label+' discards and reaps',value['outcome'] == expected and value['answer'] is None
                  and value['child_reaped'] and value['kill_to_exit_ms'] <= 250)
            preparation(app,label+' recovery preparation')
            recovery = run(app,'--recognize',(source/'samples/s03-mixed-clear.pdf').read_bytes())
            recovery['label'] = label+' recovery read'
            record['lifecycle'].append(recovery)
            check(label+' recovery read',successful(recovery) and strict_wire(recovery['answer'])
                  and recovery['answer']['status'] == 'partial'
                  and page_sections_match(recovery['answer']['text'], [1]))
        for size in [20<<20,(20<<20)+1]:
            value = run(app,'--recognize',b'X'*size)
            value['label'] = 'input bytes '+str(size)
            record['lifecycle'].append(value)
            check(value['label'], (successful(value) and value['answer']['text'] == '') if size == 20<<20
                  else value['outcome'] == 'child_failed' and value['exit_code'] == 2 and value['raw_stdout'] == '')
        record['complete'] = True
    except BaseException as error:
        record['complete'] = False
        record['error'] = str(error)
        raise
    finally:
        save()
        print(output,flush=True)
    if any(c['verdict'] != 'PASS' for c in record['checks']):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
