#!/usr/bin/env python3
"""Additional signed-bundle preparation lifecycle checks; developer evidence only."""
import argparse
import json
import os
import platform
import plistlib
import shutil
import signal
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

from check import HERE, ROOT, clean, sha

S03_SHA256 = '5ce094b565ac7f192b4d867e3fa19b6389b52a1eb2382890e2324a17f634eb91'


def process(pid):
    result = subprocess.run(['/bin/ps', '-p', str(pid), '-o', 'ppid=,stat=,lstart=,comm='],
                            capture_output=True, text=True, timeout=3)
    if not result.stdout.strip() and not result.stderr.strip() and result.returncode in (0, 1):
        return dict(pid=pid, exists=False)
    fields = result.stdout.strip().split(None, 7)
    if result.returncode or len(fields) != 8:
        return dict(pid=pid, exists=None, error=result.stderr or result.stdout)
    return dict(pid=pid, exists=True, parent_pid=int(fields[0]), state=fields[1],
                started=' '.join(fields[2:7]), command=fields[7], zombie='Z' in fields[1])


def children(parent_pid):
    result = subprocess.run(['/usr/bin/pgrep', '-P', str(parent_pid)], capture_output=True,
                            text=True, timeout=3)
    if result.returncode not in (0, 1) or result.stderr.strip():
        raise RuntimeError('Could not inspect monitor children: '+result.stderr)
    return [int(value) for value in result.stdout.split()]


def same_process(current, original):
    return current.get('exists') is True and all(
        current.get(key) == original.get(key) for key in ('pid', 'started', 'command'))


def cleanup_child(original):
    current = process(original['pid'])
    record = dict(before=current, signal_sent=False)
    if current.get('exists') is False:
        record['gone'] = True
        return record
    if not same_process(current, original):
        record.update(gone=False, reason='identity unconfirmed; no signal sent')
        return record
    try:
        os.kill(original['pid'], signal.SIGKILL)
        record['signal_sent'] = True
    except ProcessLookupError:
        pass
    deadline = time.monotonic()+2
    while time.monotonic() < deadline:
        current = process(original['pid'])
        if current.get('exists') is False:
            break
        time.sleep(0.02)
    record.update(after=current, gone=current.get('exists') is False)
    return record


def stop_monitor(monitor):
    """Best-effort cleanup after a harness failure, never a PASS condition."""
    known = []
    record = {}
    try:
        known = [process(pid) for pid in children(monitor.pid)]
    except Exception as error:
        record['child_inspection_error'] = repr(error)
    if monitor.poll() is None:
        monitor.kill()
    monitor.wait(timeout=3)
    record['children'] = [cleanup_child(child) for child in known
                          if child.get('exists') is True and child.get('parent_pid') == monitor.pid]
    return record


def successful(value):
    return (value.get('monitor_returncode') == 0 and value.get('outcome') == 'finished'
            and value.get('child_reaped') is True and value.get('exit_code') == 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--driver', required=True, type=Path, help='already-built core-check executable')
    parser.add_argument('--source-sha', required=True, help='exact source commit under test')
    parser.add_argument('--sample', type=Path, help='optional frozen s03-mixed-clear.pdf for recovery reading')
    args = parser.parse_args()
    driver = args.driver.resolve()
    if not driver.is_file() or not os.access(driver, os.X_OK):
        parser.error('--driver must be an existing executable; this script never builds it')
    sample = args.sample.resolve() if args.sample else None
    if sample and (sample.name != 's03-mixed-clear.pdf' or sha(sample) != S03_SHA256):
        parser.error('--sample must match the frozen synthetic s03 SHA-256')
    stamp = datetime.now(timezone.utc).strftime('%Y%m%dT%H%M%S%fZ')
    output = HERE/'evidence'/('lifecycle-extra-'+stamp+'.json')
    output.parent.mkdir(exist_ok=True)
    record = dict(source_sha=args.source_sha, kind='developer lifecycle check; not R4',
                  started_utc=stamp, platform=platform.platform(), machine=platform.machine(),
                  binary_sha256=sha(driver), script_sha256=sha(__file__),
                  protocol_sha256=sha(HERE/'PROTOCOL.md'), checks=[], signatures=[], scenarios={},
                  sample_sha256=sha(sample) if sample else None, complete=False,
                  source_files={str(path.relative_to(ROOT)): sha(path) for path in
                                sorted((ROOT/'crates/traeva-core/src/pdf_ocr').glob('*.rs'))},
                  limitation='New bundle IDs on this existing Mac are not fresh-device evidence.')
    with output.open('x') as stream:
        stream.write('{}\n')

    def save():
        record['last_saved_utc'] = datetime.now(timezone.utc).isoformat()
        temporary = output.with_suffix('.json.tmp')
        temporary.write_text(json.dumps(clean(record), ensure_ascii=False, indent=2)+'\n')
        temporary.replace(output)

    def check(label, condition):
        record['checks'].append(dict(label=label, verdict='PASS' if condition else 'FAIL'))
        save()
        print(label, record['checks'][-1]['verdict'], flush=True)

    def bundle(label):
        app = HERE/'target/bundles'/('lifecycle-extra-'+stamp)/(label+'.app')
        executable = app/'Contents/MacOS/ocr-core-check'
        executable.parent.mkdir(parents=True)
        shutil.copy2(driver, executable)
        identifier = 'dev.traeva.ocr.port.lifecycle.'+label+'.'+stamp
        (app/'Contents/Info.plist').write_bytes(plistlib.dumps(dict(
            CFBundleExecutable='ocr-core-check', CFBundleIdentifier=identifier,
            CFBundleName=label, CFBundlePackageType='APPL', CFBundleVersion='1',
            LSMinimumSystemVersion='13.0')))
        signed = subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', '--options',
                                 'runtime', str(app)], capture_output=True)
        verified = subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)],
                                  capture_output=True)
        details = subprocess.run(['/usr/bin/codesign', '-dvv', str(app)], capture_output=True)
        record['signatures'].append(dict(label=label, bundle_id=identifier,
            sign_returncode=signed.returncode, verify_returncode=verified.returncode,
            signature=details.stderr.decode(errors='replace'), sha256=sha(executable),
            sign_error=signed.stderr.decode(errors='replace'),
            verify_error=verified.stderr.decode(errors='replace')))
        valid = signed.returncode == 0 and verified.returncode == 0 and details.returncode == 0
        valid = valid and 'runtime' in details.stderr.decode(errors='replace')
        check(label+' strict signature and hardened runtime', valid)
        if not valid:
            raise RuntimeError('Signed bundle verification failed')
        return executable

    def run(executable, mode, budget_ms=90000, cancel_ms=None, data=b''):
        command = [str(driver), '--monitor', str(executable), mode, str(budget_ms), '384']
        if cancel_ms is not None:
            command.append(str(cancel_ms))
        monitor = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, env={})
        try:
            stdout, stderr = monitor.communicate(data, timeout=budget_ms/1000+8)
        except subprocess.TimeoutExpired:
            cleanup = stop_monitor(monitor)
            stdout, stderr = monitor.communicate(timeout=3)
            return dict(outcome='harness_timeout', answer=None, monitor_returncode=monitor.returncode,
                        raw_monitor_stdout=stdout.decode(errors='replace'),
                        monitor_stderr=stderr.decode(errors='replace'),
                        cleanup=cleanup,
                        mode=mode, budget_ms=budget_ms, cancel_ms=cancel_ms, memory_mib=384)
        except BaseException:
            stop_monitor(monitor)
            monitor.communicate(timeout=3)
            raise
        try:
            value = json.loads(stdout)
            if not isinstance(value, dict):
                raise ValueError('Monitor answer was not an object')
        except (ValueError, UnicodeDecodeError):
            value = dict(outcome='monitor_protocol_error', answer=None,
                         raw_monitor_stdout=stdout.decode(errors='replace'))
        value.update(monitor_returncode=monitor.returncode, monitor_stderr=stderr.decode(errors='replace'),
                     mode=mode, budget_ms=budget_ms, cancel_ms=cancel_ms, memory_mib=384)
        return value

    def retry(executable, scenario, label):
        value = run(executable, '--prepare')
        scenario['retry_preparation'] = value
        check(label+' retry is exact ready and reaped', successful(value) and value.get('answer') == {'status':'ready'})
        if sample:
            value = run(executable, '--recognize', budget_ms=15000, data=sample.read_bytes())
            scenario['recovery_read'] = value
            answer = value.get('answer')
            check(label+' recovery read', successful(value) and isinstance(answer, dict)
                  and set(answer) == {'status', 'text', 'note'} and answer['status'] == 'partial'
                  and bool(answer['text']))

    def interrupted_preparation(label, expected, budget_ms, cancel_ms=None):
        scenario = record['scenarios'].setdefault(label, {})
        executable = bundle(label)
        value = run(executable, '--prepare', budget_ms=budget_ms, cancel_ms=cancel_ms)
        scenario['first_preparation'] = value
        check(label+' discards and reaps', value.get('outcome') == expected and value.get('answer') is None
              and value.get('child_reaped') is True and value.get('monitor_returncode') == 0)
        latency = value.get('kill_to_exit_ms')
        check(label+' kill-to-exit within 250ms', isinstance(latency, (int, float)) and 0 <= latency <= 250)
        retry(executable, scenario, label)

    def parent_exit():
        label = 'parent-exit'
        scenario = record['scenarios'].setdefault(label, {})
        executable = bundle(label)
        monitor = subprocess.Popen([str(driver), '--monitor', str(executable), '--prepare', '90000', '384'],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, env={})
        scenario['monitor_pid'] = monitor.pid
        original = None
        try:
            discovery_deadline = time.monotonic()+2
            while time.monotonic() < discovery_deadline and monitor.poll() is None:
                found = children(monitor.pid)
                if len(found) == 1:
                    snapshot = process(found[0])
                    if (snapshot.get('exists') is True and not snapshot.get('zombie')
                            and snapshot.get('parent_pid') == monitor.pid):
                        original = snapshot
                        break
                time.sleep(0.01)
            scenario['child_before_parent_exit'] = original
            check(label+' live child confirmed', original is not None)
            if original is None:
                raise RuntimeError('No live preparation child was confirmed before parent exit')
            stopped = time.monotonic()
            monitor.kill()
            monitor.wait(timeout=2)
            scenario['monitor_returncode'] = monitor.returncode
            check(label+' parent killed and waited', monitor.returncode == -signal.SIGKILL)
            scenario['observations'] = []
            gone = False
            while time.monotonic()-stopped <= 2:
                snapshot = process(original['pid'])
                snapshot['after_parent_kill_ms'] = (time.monotonic()-stopped)*1000
                scenario['observations'].append(snapshot)
                save()
                if snapshot.get('exists') is False:
                    gone = True
                    break
                time.sleep(0.01)
            scenario['child_gone_ms'] = (time.monotonic()-stopped)*1000 if gone else None
            check(label+' watchdog child absent within 2s', gone and scenario['child_gone_ms'] <= 2000)
            if not gone:
                scenario['failed_case_cleanup'] = cleanup_child(original)
                save()
        finally:
            if monitor.poll() is None:
                scenario['monitor_cleanup'] = stop_monitor(monitor)
            if original and process(original['pid']).get('exists') is not False:
                scenario['final_cleanup'] = cleanup_child(original)
            stdout, stderr = monitor.communicate(timeout=3)
            scenario['monitor_stdout'] = stdout.decode(errors='replace')
            scenario['monitor_stderr'] = stderr.decode(errors='replace')
            save()
        retry(executable, scenario, label)

    save()
    cases = [
        ('prepare-cancel', lambda: interrupted_preparation('prepare-cancel', 'cancelled', 90000, 1000)),
        ('prepare-timeout', lambda: interrupted_preparation('prepare-timeout', 'time_limit', 1000)),
        ('parent-exit', parent_exit),
    ]
    for label, case in cases:
        try:
            case()
        except BaseException as error:
            record['scenarios'].setdefault(label, {})['error'] = repr(error)
            check(label+' scenario completed', False)
            if isinstance(error, (KeyboardInterrupt, SystemExit)):
                raise
    record['complete'] = True
    save()
    print(output, flush=True)
    if any(check['verdict'] != 'PASS' for check in record['checks']):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
