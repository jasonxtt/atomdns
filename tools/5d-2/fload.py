"""Open-loop steady UDP load with a scheduled event timeline (5D-2).

Like tools/5d-1/load.py (absolute schedule, 128 sockets, no catch-up bursts) but
for one steady stage with a long client deadline, so a failing upstream's
SERVFAIL or timeout is observed instead of being cut off at 500 ms. Every row
keeps its corpus group; every answer is classified against the oracle:

  correct   semantic(answer) == oracle and TTLs in 0..300
  servfail  rcode 2 with the right question (an explicit, well-formed failure)
  wrong     any other decoded answer (an incorrect DNS result)
  malformed an undecodable packet that matched a request
  timeout   no answer within the client deadline
  send_error / unsent

Events (fault injection, management requests) run on their own thread at
absolute offsets from the first planned send, so a slow management call never
delays the schedule. Python standard library only.
"""
import argparse
import bisect
import json
import gzip
import multiprocessing
import os
import pathlib
import random
import selectors
import socket
import sys
import threading
import time
import urllib.error
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'e2e-1'))
sys.path.insert(0, str(HERE.parent / '5d-1'))
from dnswire import decode, question  # noqa: E402
from load import sample, semantic  # noqa: E402


def classify(value, expected, alternatives=()):
    """Pure: the status of one decoded answer against its oracle. `alternatives`
    are other semantically acceptable answers (a configured fallback's); a match
    is reported as `fallback`, never folded into `correct`."""
    ttl_ok = all(0 <= rr[4] <= 300 for section in ('answers', 'authority', 'additional') for rr in value[section])
    shape = semantic(value)
    if shape == expected and ttl_ok:
        return 'correct'
    if shape in alternatives and ttl_ok:
        return 'fallback'
    if value['rcode'] == 2 and not value['answers'] and not value['authority']:
        return 'servfail'
    return 'wrong'


def read_udp_counters(pid):
    """Kernel UDP counters of the host's network namespace (datagrams dropped by a
    full socket buffer are `RcvbufErrors`; they are never seen by the process)."""
    try:
        lines = pathlib.Path(f'/proc/{pid}/net/snmp').read_text().splitlines()
    except OSError:
        return {}
    counters = {}
    for header, values in zip(lines, lines[1:]):
        if header.startswith('Udp:') and values.startswith('Udp:'):
            counters = dict(zip(header.split()[1:], map(int, values.split()[1:])))
    # Drops of the host's own DNS socket(s) on port 53 (the last column of /proc/net/udp).
    drops = 0
    for name in ('udp', 'udp6'):
        try:
            for line in pathlib.Path(f'/proc/{pid}/net/{name}').read_text().splitlines()[1:]:
                fields = line.split()
                if int(fields[1].rsplit(':', 1)[1], 16) == 53:
                    drops += int(fields[-1])
        except OSError:
            pass
    counters['Drops53'] = drops
    return counters


def read_connections(pid):
    counts = {}
    for name in ('tcp', 'tcp6'):
        try:
            lines = pathlib.Path(f'/proc/{pid}/net/{name}').read_text().splitlines()[1:]
        except OSError:
            continue
        for line in lines:
            fields = line.split()
            port = int(fields[2].rsplit(':', 1)[1], 16)
            key = f'{fields[3]}:{port}'
            counts[key] = counts.get(key, 0) + 1
    return counts


def http_event(port, event):
    body = event.get('body')
    data = None if body is None else (body if isinstance(body, str) else json.dumps(body)).encode()
    request = urllib.request.Request(f'http://127.0.0.1:{port}{event["path"]}', data=data,
                                     method=event.get('method', 'GET'),
                                     headers={'Content-Type': 'application/json'})
    began = time.monotonic()
    try:
        with urllib.request.urlopen(request, timeout=event.get('timeout', 120)) as response:
            status, text = response.status, response.read().decode(errors='replace')
    except urllib.error.HTTPError as error:
        status, text = error.code, error.read().decode(errors='replace')
    except OSError as error:
        status, text = 0, repr(error)
    return dict(status=status, seconds=time.monotonic() - began, body=text[:400])


def udp_probe(event):
    """One query to `port`; records rcode, answers and the time it took."""
    began = time.monotonic()
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.settimeout(event.get('timeout', 3))
        sock.connect(('127.0.0.1', event['port']))
        sock.send(question(event['name'], event.get('type', 1), 0x5d2))
        try:
            value = decode(sock.recv(65535))
        except socket.timeout:
            return dict(answered=False, seconds=time.monotonic() - began)
        except OSError as error:
            return dict(answered=False, error=repr(error), seconds=time.monotonic() - began)
    return dict(answered=True, rcode=value['rcode'], answers=[a[3] for a in value['answers']],
                seconds=time.monotonic() - began)


def run_events(spec, start, root, port, log):
    """Starts every event at its absolute offset, each on its own thread so a slow
    management call or poll never delays the next event; every action is recorded."""
    workers = []
    for event in sorted(spec.get('events', []), key=lambda e: e['at']):
        delay = start + event['at'] - time.monotonic()
        if delay > 0:
            time.sleep(delay)
        worker = threading.Thread(target=execute, args=(event, start, root, port, log), daemon=True)
        worker.start()
        workers.append(worker)
    for worker in workers:
        worker.join()


def execute(event, start, root, port, log):
    began = time.monotonic()
    record = dict(event, started=began, planned=start + event['at'])
    try:
        if event['action'] == 'faults':
            path, temporary = root / 'faults.json', root / 'faults.json.tmp'
            temporary.write_text(json.dumps(event['table']))
            os.replace(temporary, path)
            record['result'] = 'written'
        elif event['action'] == 'http':
            record['result'] = http_event(port, event)
        elif event['action'] == 'poll':
            # GET `path` every `interval` seconds until `until` appears in the body.
            deadline_at = time.monotonic() + event.get('timeout', 120)
            polls = []
            while time.monotonic() < deadline_at:
                answer = http_event(port, dict(event, method='GET', body=None, timeout=10))
                polls.append(dict(t=time.monotonic(), status=answer['status'], body=answer['body'][:200]))
                if event['until'] in answer['body']:
                    break
                time.sleep(event.get('interval', 2))
            record['result'] = dict(polls=len(polls), last=polls[-1] if polls else None,
                                    reached=bool(polls) and event['until'] in polls[-1]['body'])
        elif event['action'] == 'probe':
            record['result'] = udp_probe(event)
        elif event['action'] == 'snapshot':
            record['result'] = proc_snapshot(event)
        elif event['action'] == 'note':
            record['result'] = 'note'
        else:
            record['result'] = 'unknown action'
    except BaseException as error:  # the record must survive any failure
        record['result'] = repr(error)
    record['finished'] = time.monotonic()
    log.append(record)


def proc_snapshot(event):
    """The host's resident memory by kind (KiB) from /proc: VmRSS, VmHWM (peak since start or
    the last reset) and the Rss of `[heap]`, other anonymous, stack and file-backed mappings.
    `reset_hwm` then resets VmHWM, so a later snapshot reports the peak since this one."""
    proc = pathlib.Path(f"/proc/{event['pid']}")
    status = dict(line.split(':', 1) for line in (proc / 'status').read_text().splitlines() if ':' in line)
    kinds, current = dict(heap=0, anon=0, stack=0, file=0), None
    for line in (proc / 'smaps').read_text().splitlines():
        fields = line.split()
        if fields and '-' in fields[0] and not fields[0].endswith(':'):
            name = fields[5] if len(fields) > 5 else ''
            current = ('heap' if name == '[heap]' else 'stack' if name.startswith('[stack') else
                       'anon' if not name or name.startswith('[anon') else 'file' if name.startswith('/') else None)
        elif fields[:1] == ['Rss:'] and current:
            kinds[current] += int(fields[1])
    result = dict(rss_kib=int(status['VmRSS'].split()[0]), hwm_kib=int(status['VmHWM'].split()[0]),
                  rss_by_kind_kib=kinds, anon_huge_kib=sum(
                      int(line.split()[1]) for line in (proc / 'smaps_rollup').read_text().splitlines()
                      if line.startswith('AnonHugePages:')))
    if event.get('reset_hwm'):
        (proc / 'clear_refs').write_text('5')
    return result


def sample_in_process(pids, stop, ready, output, cpu=None):
    """Read proc outside the sending loop; all processes share monotonic time."""
    samples = []
    if cpu is not None:os.sched_setaffinity(0,{cpu})
    nice = os.nice(10)
    pathlib.Path(output).with_name('sampler.json').write_text(json.dumps(dict(sampler_pid=os.getpid(), sender_pid=pids['sender'], sampler_nice=nice,affinity=sorted(os.sched_getaffinity(0)))))
    pids = dict(pids, sampler=os.getpid())
    next_connections = 0.0
    try:
        while not stop.is_set():
            began = time.monotonic()
            taken = dict(sample(pids))
            if began >= next_connections:
                taken['connections'] = read_connections(pids['sut'])
                taken['udp'] = read_udp_counters(pids['sut'])
                next_connections = began + 2
            samples.append(taken)
            ready.set()
            stop.wait(max(0, began + .5 - time.monotonic()))
        samples.append(dict(sample(pids)))
    finally:
        pathlib.Path(output).write_text(json.dumps(samples))


def measure(spec):
    out = pathlib.Path(spec['output'])
    out.mkdir(parents=True, exist_ok=True)
    rows = json.loads(pathlib.Path(spec['corpus']).read_text())
    observation = spec.get('observation_only', False)
    precise = spec.get('precise_timer', False)
    separate = spec.get('sampling_process', False)
    sampler_cpu = spec.get('sampler_cpu')
    extra_pids = spec.get('extra_pids', {})
    retain_wire = spec.get('retain_wire', False)
    if (extra_pids or retain_wire) and not observation:raise ValueError('extra resources/wire retention require observation mode')
    if separate and not observation:raise ValueError('separate sampler is currently restricted to observations')
    if sampler_cpu is not None and (not separate or type(sampler_cpu) is not int or sampler_cpu < 0):raise ValueError('sampler_cpu requires a separate sampler and nonnegative CPU')
    if not isinstance(extra_pids,dict) or any(k in {'sut','mock','sender','sampler'} or type(v) is not int or v <= 0 for k,v in extra_pids.items()):raise ValueError('invalid extra resource PID')
    if precise and (not observation or not hasattr(os, 'timerfd_create')):
        raise ValueError('precise timer requires Linux timerfd and observation mode')
    oracle = [] if observation else json.loads(pathlib.Path(spec['oracle']).read_text())
    if observation:
        sys.path.insert(0,str(HERE.parent/'memory-validation'))
        from real_metrics import observed_status
    raw_replies = gzip.open(out/'responses.jsonl.gz','wt') if observation else None
    expected = {r['index']: r['semantic'] for r in oracle}
    alternatives = {r['index']: tuple(r.get('alt', ())) for r in oracle}
    rate, duration, deadline = spec['rate'], spec['duration'], spec['deadline_s']
    count = round(rate * duration)
    if spec.get('mode', 'sequential') == 'weighted':
        cumulative, total = [], 0
        for row in rows:
            total += row['weight']
            cumulative.append(total)
        rng = random.Random(spec.get('seed', 501))
        picks = [bisect.bisect(cumulative, rng.random() * total) for _ in range(count)]
        picks = [min(p, len(rows) - 1) for p in picks]
    else:
        if count > len(rows):
            raise ValueError('sequential corpus shorter than the schedule')
        picks = list(range(count))
    wires, data = [], []
    for seq, idx in enumerate(picks):
        if seq // 128 >= 65536:
            raise ValueError('DNS ID envelope exceeded')
        row = rows[idx]
        wires.append(question(row['name'], row['type'], seq // 128))
        data.append(dict(seq=seq, index=idx, group=row.get('group', 'prod'), planned=seq / rate, sent=None,
                         finished=None, status='unsent', rcode=None))
    selector, sockets = selectors.DefaultSelector(), []
    for i in range(128):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.connect(('127.0.0.1', spec.get('dns_port', 53)))
        sock.setblocking(False)
        selector.register(sock, selectors.EVENT_READ, i)
        sockets.append(sock)
    pids = dict(sut=spec['pid'], mock=spec['mock_pid'], sender=os.getpid(), **extra_pids)
    sampler = sampler_stop = None
    if separate:
        context = multiprocessing.get_context('spawn')
        sampler_stop, sampler_ready = context.Event(), context.Event()
        sampler = context.Process(target=sample_in_process,
                                  args=(pids, sampler_stop, sampler_ready, str(out/'resources.json'),sampler_cpu))
        sampler.daemon = True
        sampler.start()
        if not sampler_ready.wait(10):
            sampler_stop.set(); sampler.terminate(); sampler.join()
            raise RuntimeError('resource sampler not ready')
    samples, active, events, orphan = [], {}, [], 0
    start = time.monotonic() + 1
    for row in data:
        row['planned'] += start
    if spec.get('start_file'):
        pathlib.Path(spec['start_file']).write_text(json.dumps(dict(start=start)))
    thread = threading.Thread(target=run_events, args=(spec, start, pathlib.Path(spec['root']), spec['api_port'], events), daemon=True)
    thread.start()
    if not separate:
        samples.append(dict(sample(pids), connections=read_connections(spec['pid']), udp=read_udp_counters(spec['pid'])))
    next_sample = next_connections = 0.0
    cursor = 0
    end = start + duration
    stop = end + deadline + 0.5
    send_timer = None
    try:
        if precise:
            send_timer = os.timerfd_create(time.CLOCK_MONOTONIC, flags=os.TFD_NONBLOCK | os.TFD_CLOEXEC)
            selector.register(send_timer, selectors.EVENT_READ, 'schedule')
        while time.monotonic() < stop:
            now = time.monotonic()
            if now >= next_sample:
                # The connection table of a busy host is expensive to read: every 2 s only.
                if not separate:
                    taken = dict(sample(pids))
                    if now >= next_connections:
                        taken['connections'] = read_connections(spec['pid'])
                        taken['udp'] = read_udp_counters(spec['pid'])
                        next_connections = now + 2
                    samples.append(taken)
                next_sample = now + .5
                # A pending row older than the client deadline is a timeout (kept in the denominator).
                for key, row in list(active.items()):
                    if row['planned'] < now - deadline:
                        row['status'] = 'timeout'
                        del active[key]
            while cursor < len(data) and data[cursor]['planned'] <= now:
                row = data[cursor]
                key = (row['seq'] % 128, row['seq'] // 128)
                # Open loop: a slot missed by more than 20 ms is skipped (reported as `unsent`),
                # never sent as part of a catch-up burst.
                if now - row['planned'] <= .02:
                    try:
                        sockets[key[0]].send(wires[cursor])
                        row['sent'] = time.monotonic()
                        row['status'] = 'pending'
                        active[key] = row
                    except OSError:
                        row['status'] = 'send_error'
                cursor += 1
                now = time.monotonic()
            wait_until = data[cursor]['planned'] if cursor < len(data) else end + deadline
            if send_timer is not None:
                os.timerfd_settime(send_timer, flags=os.TFD_TIMER_ABSTIME, initial=wait_until if cursor < len(data) else 0)
            timeout = .005 if precise else max(0, min(.005, wait_until - time.monotonic()))
            for event, _ in selector.select(timeout):
                if event.data == 'schedule':
                    try:
                        os.read(send_timer, 8)
                    except BlockingIOError:
                        pass
                    continue
                for _ in range(32):
                    try:
                        wire = event.fileobj.recv(65535)
                    except BlockingIOError:
                        break
                    except OSError:
                        break
                    finished = time.monotonic()
                    try:
                        value = decode(wire)
                    except (ValueError, IndexError, KeyError, OSError):
                        orphan += 1
                        continue
                    row = active.pop((event.data, value['id']), None)
                    if row is None:
                        orphan += 1
                        continue
                    row['finished'] = finished
                    row['rcode'] = value['rcode']
                    if observation:
                        item=rows[row['index']]
                        row['status']=observed_status(value,item['name'],item['type'])
                        raw_replies.write(json.dumps(dict(seq=row['seq'],index=row['index'],finished=finished,dns=value,**({'wire_hex':wire.hex()} if retain_wire else {})))+'\n')
                    else:
                        row['status'] = classify(value, expected[row['index']], alternatives[row['index']])
                    if row['status'] == 'wrong':
                        row['answer'] = [value['rcode'], value['questions'], value['answers'][:4]]
        for row in active.values():
            row['status'] = 'timeout'
        if not separate:
            samples.append(dict(sample(pids), connections=read_connections(spec['pid']), udp=read_udp_counters(spec['pid'])))
    finally:
        for sock in sockets:
            sock.close()
        selector.close()
        if send_timer is not None:
            os.close(send_timer)
        if raw_replies is not None:
            raw_replies.close()
        if sampler is not None:
            sampler_stop.set()
            sampler.join(timeout=10)
            if sampler.is_alive():
                sampler.terminate(); sampler.join()
                raise RuntimeError('resource sampler did not stop')
            if sampler.exitcode != 0:
                raise RuntimeError('resource sampler failed')
            samples = json.loads((out/'resources.json').read_text())
    thread.join(timeout=5)
    # Unmatched packets (wrong ID, unknown key, late after timeout) are counted, not hidden.
    if observation:
        with gzip.open(out/'requests.jsonl.gz','wt') as target:
            for row in data:target.write(json.dumps(row)+'\n')
    else:
        (out / 'requests.jsonl').write_text(''.join(json.dumps(r) + '\n' for r in data))
    (out / 'resources.json').write_text(json.dumps(samples))
    (out / 'events.json').write_text(json.dumps(events))
    (out / 'summary.json').write_text(json.dumps(dict(start=start, duration=duration, rate=rate, deadline_s=deadline,
                                                      rows=len(data), orphan_packets=orphan, events=len(events), correctness_checked=not observation, sampling_process=separate, precise_timer=precise,retain_wire=retain_wire)))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('spec', type=pathlib.Path)
    measure(json.loads(parser.parse_args().spec.read_text()))
