#!/usr/bin/env python3
"""Private, closed-loop state-aware functional replay. Never timing acceptance."""
import argparse
import hashlib
import importlib.util
import json
import pathlib
import random
import socket
import sys
import time
import urllib.parse
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
from state_model import compare  # noqa: E402


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def http(port, path):
    with urllib.request.urlopen(f'http://127.0.0.1:{port}{path}', timeout=5) as response:
        return json.load(response)


def learner_rows(port, tags, name=None):
    rows = []
    for tag in tags:
        offset = 0
        while True:
            query = urllib.parse.urlencode(dict(limit=1000, offset=offset, q=name or '', sort='last'))
            value = http(port, '/plugins/' + urllib.parse.quote(tag, safe='') + '/entries?' + query)
            page = value['entries']
            if not page and offset < value['total']:
                raise ValueError('learner pagination stopped early')
            rows.extend(dict(row, learner=tag) for row in page if name is None or row['name'] == name)
            offset += len(page)
            if offset >= value['total']:
                break
    return rows


ROUTE_FIELDS = ('query_name', 'query_type', 'query_class', 'client_ip', 'response_code',
                'response_flags', 'answer_details_status', 'answer_decode_error', 'domain_set',
                'effective_tag', 'matched_group', 'final_sequence', 'final_upstream',
                'upstream_targets', 'selected_upstream', 'matched_rule_source')


def valid_ttls(dns, ceiling=300):
    return all(0 <= rr[4] <= ceiling for key in ['answers', 'authority', 'additional'] for rr in dns[key])


def worker(spec):
    sys.path.insert(0, str(HERE.parent/'5d-1'))
    from load import semantic
    from dnswire import question, decode
    sequence = json.loads(pathlib.Path(spec['sequence']).read_text())
    output = pathlib.Path(spec['output'])
    port, tags = spec['api_port'], spec['learners']
    initial = learner_rows(port, tags)
    save(output/'initial-learner.json', initial)
    save(output/'learner-status.json', {t: http(port, '/plugins/'+urllib.parse.quote(t, safe='')+'/status') for t in tags})
    prior = None
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock, (output/'trace.jsonl').open('w') as trace:
        sock.connect(('127.0.0.1', spec.get('dns_port', 53)))
        sock.settimeout(5)
        for index, item in enumerate(sequence):
            started = time.time()
            before = learner_rows(port, tags, item['name'])
            sock.send(question(item['name'], item['type'], index % 65536))
            dns = decode(sock.recv(65535))
            if dns['id'] != index % 65536 or dns['questions'] != [(item['name']+'.', item['type'], 1)] or dns['rcode'] == 2:
                raise ValueError('wrong DNS question/id or SERVFAIL')
            if not valid_ttls(dns, spec.get('ttl_ceiling', 300)):
                raise ValueError('DNS TTL outside existing oracle bounds')
            deadline = time.monotonic()+5
            while True:
                logs = http(port, '/api/v2/audit/logs?limit=1')['logs']
                audit = logs[0] if logs else None
                if audit and audit['trace_id'] != prior:
                    break
                if time.monotonic() >= deadline:
                    raise ValueError('missing terminal audit')
                time.sleep(.01)
            if audit['query_name'] != item['name'] or audit['query_type'] != {1:'A',28:'AAAA'}[item['type']]:
                raise ValueError('terminal audit belongs to another query')
            prior = audit['trace_id']
            after = learner_rows(port, tags, item['name'])
            row = dict(index=index, corpus_index=item['corpus_index'], before=before, after=after,
                       started=started, finished=time.time(), dns=semantic(dns),
                       route={k: audit.get(k) for k in ROUTE_FIELDS}, raw_dns=dns, raw_audit=audit)
            trace.write(json.dumps(row, ensure_ascii=False)+'\n')
            trace.flush()
    save(output/'final-learner.json', learner_rows(port, tags))


def load_runner():
    spec = importlib.util.spec_from_file_location('memory_validation_runner', HERE/'run.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run(args):
    from state_model import state
    runner = load_runner()
    from load import corpus
    from prepare import definitions
    args.output.mkdir(mode=0o700, parents=True)
    manifest = json.loads((args.work/'input-manifest.json').read_text())
    actual = {str(p.relative_to(args.work/'native')): digest(p) for p in (args.work/'native').rglob('*') if p.is_file()}
    if actual != manifest['prepared']:
        raise ValueError('prepared input changed')
    tags = [d['tag'] for d in definitions(args.work/'native') if d['type']=='route_learner']
    if not tags:
        raise ValueError('state-aware learner replay requires configured learners')
    values = corpus((args.work/'native/gen/top_domains.txt').read_text())
    indices = list(range(len(values)))
    if args.limit is not None:
        indices = indices[:args.limit]
    indices = sorted(set(indices + args.include_index + [i for i,v in enumerate(values) if v['kind'] != 'production']))
    if not indices or min(indices) < 0 or max(indices) >= len(values):
        raise ValueError('invalid corpus selection')
    rng = random.Random(args.seed)
    sequence = []
    for _ in range(2):
        order = indices[:]
        rng.shuffle(order)
        sequence.extend(dict(values[i], corpus_index=i) for i in order)
    save(args.output/'sequence.json', sequence)
    identity = dict(scope='closed-loop state-aware DNS/learner/routing projection only',
                    whole_project_accepted=False, timing_eligible=False,
                    supplementary=args.limit is not None, seed=args.seed, passes=2, planned=len(sequence),
                    corpus_questions=len(values), selected_questions=len(indices), learners=tags,
                    prepared_manifest_sha256=digest(args.work/'input-manifest.json'),
                    sequence_sha256=digest(args.output/'sequence.json'),
                    binaries={k: digest(v) for k,v in [('base',args.base), ('control',args.base), ('candidate',args.candidate)]},
                    tools_sha256={p.name:digest(p) for p in [HERE/'state_replay.py', HERE/'state_model.py', HERE/'run.py']})
    save(args.output/'identity.json', identity)
    traces, initial, final, cleanup = {}, {}, {}, {}
    for role, binary in [('base', args.base), ('control', args.base), ('candidate', args.candidate)]:
        output = args.output/role
        with runner.ValidationEnv(args.work, 'native', binary.resolve(), output) as env:
            save(output/'worker-spec.json', dict(api_port=env.api_port, learners=tags,
                 sequence=str((args.output/'sequence.json').resolve()), output=str(output.resolve())))
            child = env.spawn(1, ['python3', HERE/'state_replay.py', '--worker', output/'worker-spec.json'], 'state-worker.log')
            if child.wait() != 0:
                raise RuntimeError('state replay worker failed: '+role)
        traces[role] = [json.loads(line) for line in (output/'trace.jsonl').read_text().splitlines()]
        initial[role] = json.loads((output/'initial-learner.json').read_text())
        final[role] = json.loads((output/'final-learner.json').read_text())
        receipt = json.loads((output/'cleanup.json').read_text())
        receipt['existing_services_unchanged'] = json.loads((output/'existing-service-guard.json').read_text())['unchanged']
        cleanup[role] = receipt
    verdicts = {}
    for role in ['control', 'candidate']:
        result = compare(traces['base'], traces[role], len(sequence), initial['base'], initial[role])
        if state(final['base']) != state(final[role]):
            result = dict(status='fail', reason='final learner population differs', timing_eligible=False, whole_project_accepted=False)
        verdicts[role] = result
    if not all(all(r.get(k) is True for k in ['all_owned_processes_exited','namespace_deleted','original_service_unchanged','existing_services_unchanged']) for r in cleanup.values()):
        verdicts['cleanup'] = dict(status='incomplete')
    report = dict(identity=identity, verdicts=verdicts, cleanup=cleanup,
                  raw_sha256={str(p.relative_to(args.output)):digest(p) for p in args.output.glob('*/*.json*')},
                  status='pass' if all(v['status']=='pass' for v in verdicts.values()) else 'not_passed',
                  whole_project_accepted=False, timing_eligible=False)
    save(args.output/'report.json', report)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--worker', type=pathlib.Path)
    p.add_argument('--work', type=pathlib.Path)
    p.add_argument('--output', type=pathlib.Path)
    p.add_argument('--base', type=pathlib.Path)
    p.add_argument('--candidate', type=pathlib.Path)
    p.add_argument('--limit', type=int)
    p.add_argument('--include-index', type=int, action='append', default=[])
    p.add_argument('--seed', type=int, default=501)
    a = p.parse_args()
    if a.worker:
        worker(json.loads(a.worker.read_text()))
    else:
        if any(v is None for v in [a.work,a.output,a.base,a.candidate]) or a.limit is not None and a.limit <= 0:
            p.error('work/output/base/candidate required; limit must be positive')
        run(a)
