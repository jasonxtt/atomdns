#!/usr/bin/env python3
"""Native-only isolated full-config before/after validation runner.

Uses the existing 5D fixture, sender, strict full-section DNS oracle and cleanup
receipts. Raw inputs/results remain private. This tool never builds or deploys.
An explicit --smoke shortens timing and is NEVER acceptance evidence.
"""
import argparse
import collections
import hashlib
import json
import pathlib
import statistics
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / '5d-2'))
sys.path.insert(0, str(HERE.parent / '5d-1'))
from harness import Env  # noqa: E402
from load import corpus  # noqa: E402
import analyze  # noqa: E402
from evaluate import event_failed, valid_dns_outcomes, static_oracle_block_reason  # noqa: E402
from fload import proc_snapshot  # noqa: E402

def existing_service_state():
    # The older harness checks the atomdns unit. This host also runs a
    # differently named mosdns-rust service, so protect all existing product
    # processes by PID/start/image identity, without reading their configs.
    identities = {}
    for proc in pathlib.Path('/proc').iterdir():
        if not proc.name.isdigit():
            continue
        try:
            comm = (proc/'comm').read_text().strip()
            if comm in {'atomdns', 'rtdns', 'mosdns', 'mosdns-rust'}:
                fields = (proc/'stat').read_text().rsplit(')',1)[1].split()
                image = (proc/'exe').stat()
                identities[proc.name] = dict(comm=comm, start_ticks=int(fields[19]),
                                             image_device=image.st_dev, image_inode=image.st_ino)
        except (FileNotFoundError, ProcessLookupError):
            continue
    return identities


class ValidationEnv(Env):
    def __enter__(self):
        self.existing_before = existing_service_state()
        return super().__enter__()

    def __exit__(self, *args):
        try:
            return super().__exit__(*args)
        finally:
            after = existing_service_state()
            unchanged = after == self.existing_before
            save(self.output/'existing-service-guard.json',
                 dict(before=self.existing_before, after=after, unchanged=unchanged))
            if not unchanged:
                raise RuntimeError('an existing product service process changed')


def point(pid):
    value = proc_snapshot(dict(pid=pid))
    lines = pathlib.Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines()
    value["pss_kib"] = int(next(line.split()[1] for line in lines if line.startswith("Pss:")))
    return value



def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')


def launch_sender(env, spec, output):
    spec_path = output / 'sender-spec.json'
    spec.update(pid=env.host.pid, mock_pid=env.mock.pid, root=str(env.root), api_port=env.api_port,
                output=str(output / 'load'), deadline_s=.5)
    save(spec_path, spec)
    child = env.spawn(1, ['python3', HERE.parent / '5d-2/fload.py', spec_path], output.name + '-sender.log')
    if child.wait() != 0:
        raise RuntimeError('sender failed')
    if env.host.poll() is not None:
        raise RuntimeError('host exited during load')
    samples = json.loads((output / 'load/resources.json').read_text())
    rows = [json.loads(s) for s in (output / 'load/requests.jsonl').read_text().splitlines()]
    summary = json.loads((output / 'load/summary.json').read_text())
    return rows, samples, summary['start']


def events(scenario, warmup):
    def http(at, method, path, body=None):
        return dict(at=warmup+at, action='http', method=method, path=path, body=body)
    if scenario == 'settings':
        return [http(t, 'POST', '/api/v1/overrides', dict(socks5='127.0.0.1:' + ('17892' if i % 2 == 0 else '17891'), ecs='', replacements=[]))
                for i, t in enumerate([20, 60, 100, 140, 180, 220])]
    if scenario == 'groups':
        group = dict(name='memory-test', listen_port=3999, custom_port_only=True,
                     upstream_sources=[dict(kind='group', plugin_tag='foreign')])
        return [http(t, 'POST' if i % 2 == 0 else 'DELETE', '/api/v1/special-groups' + ('' if i % 2 == 0 else '/50'), group if i % 2 == 0 else None)
                for i, t in enumerate([20, 60, 100, 140, 180, 220])]
    if scenario == 'reads':
        return [http(t, 'GET', path) for t in range(10, 291, 20) for path in
                ['/api/v2/audit/stats/windows', '/api/v2/audit/logs?q=example&limit=500',
                 '/api/v2/audit/rank/domain', '/api/v2/audit/rank/client', '/api/v2/audit/rank/slowest']]
    if scenario == 'reload':
        # Native rule-file POST publishes a new matcher generation. Keep the
        # payload semantically identical so the oracle remains independent.
        return []  # attached by run() from the immutable block-list input
    return []


def run(args):
    args.output.mkdir(mode=0o700, parents=True)
    manifest = json.loads((args.work/'input-manifest.json').read_text())
    if {str(p.relative_to(args.work/'native')) for p in (args.work/'native').rglob('*') if p.is_file()} != set(manifest['prepared']):
        raise ValueError('prepared input file inventory changed')
    for name, expected in manifest['prepared'].items():
        path = args.work/'native'/name
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != expected:
            raise ValueError('prepared input changed after fingerprinting: '+name)
    capacity = json.loads((args.work/'native/webinfo/audit_settings.json').read_text())['capacity']
    args.prefill = capacity if args.prefill is None else args.prefill
    if not args.smoke and args.prefill != capacity:
        raise ValueError('prefill must match the frozen capacity')
    # The oracle is unsafe even with an empty saved file: configured learning
    # can change routing during characterization, prefill and measured replay.
    if not args.oracle_only and not args.smoke and not args.diagnostic:
        reason = static_oracle_block_reason(manifest)
        if reason is not None:
            save(args.output/'preflight.json', dict(status='blocked', reason=reason, whole_project_accepted=False))
            raise ValueError('state-aware oracle validation is required before formal timing: '+reason)
    paths = {name: pathlib.Path(path).resolve() for name, path in (v.split('=', 1) for v in args.binary)}
    roles = ['base'] if args.oracle_only else list(paths)
    if not args.oracle_only and not args.smoke and not args.diagnostic and (len(roles) != 2 or roles[0] != 'base'):
        raise ValueError('formal timing requires exactly one baseline/candidate pair')
    top_path = args.top or args.work / 'native/gen/top_domains.txt'
    values = corpus(top_path.read_text())
    corpus_path = args.output / 'corpus.json'
    save(corpus_path, values)
    identity = dict(excluded_from_acceptance=args.smoke or args.diagnostic, binaries={k: hashlib.sha256(p.read_bytes()).hexdigest() for k,p in paths.items()},
                    corpus_sha256=hashlib.sha256(corpus_path.read_bytes()).hexdigest(),
                    top_source='explicit' if args.top is not None else 'default',
                    top_sha256=hashlib.sha256(top_path.read_bytes()).hexdigest(), rate=args.rate, scenario=args.scenario, audit_capacity=capacity, prefill=args.prefill,
                    prepared_manifest_sha256=hashlib.sha256((args.work/'input-manifest.json').read_bytes()).hexdigest(),
                    tools_sha256={str(p.relative_to(HERE.parent)):hashlib.sha256(p.read_bytes()).hexdigest() for p in
                                  [HERE/'run.py', HERE/'evaluate.py', HERE.parent/'5d-2/fload.py', HERE.parent/'5d-2/harness.py',
                                   HERE.parent/'5d-2/analyze.py', HERE.parent/'5d-1/load.py', HERE.parent/'e2e-1/mock.py', HERE.parent/'e2e-1/dnswire.py']},
                    warmup=0 if args.diagnostic else 2 if args.smoke else 60, duration=1 if args.diagnostic else 3 if args.smoke else 300)
    save(args.output / 'identity.json', identity)
    oracle = args.oracle or args.output / 'oracle.json'
    if args.oracle_only:
        with ValidationEnv(args.work, 'native', paths['base'], args.output / 'oracle-run') as env:
            child = env.spawn(1, ['python3', HERE.parent / '5d-1/load.py', 'oracle', '--corpus', corpus_path, '--oracle', oracle], 'oracle-client.log')
            if child.wait() != 0:
                raise RuntimeError('baseline oracle failed')
        return
    if not oracle.is_file():
        raise ValueError('use an independently generated baseline oracle')
    save(args.output / 'oracle-identity.json', dict(sha256=hashlib.sha256(oracle.read_bytes()).hexdigest()))
    summaries = []
    for pair in range(1 if args.smoke or args.diagnostic else 3):
        for role in roles if pair % 2 == 0 else list(reversed(roles)):
            output = args.output / f'r{pair+1}-{role}'
            with ValidationEnv(args.work, 'native', paths[role], output) as env:
                before = point(env.host.pid)
                prefill_dns = dict(planned=0, sent=0, healthy=0, statuses={})
                if args.prefill and not args.smoke:
                    fill = output / 'fill'
                    fill.mkdir()
                    fill_rows, _, _ = launch_sender(env, dict(corpus=str(corpus_path), oracle=str(oracle), mode='weighted', seed=400+pair,
                                            rate=2000, duration=(args.prefill+10000)/2000, events=[]), fill)
                    prefill_dns = dict(planned=args.prefill+10000, sent=sum(r['sent'] is not None for r in fill_rows),
                                       healthy=sum(analyze.healthy(r) for r in fill_rows),
                                       statuses=dict(collections.Counter(r['status'] for r in fill_rows)))
                    save(output/'prefill-summary.json', prefill_dns)
                    if not args.diagnostic and not valid_dns_outcomes(prefill_dns):
                        raise RuntimeError('prefill DNS correctness failed; main measurement not started')
                    stats = json.loads(env.api('GET', '/api/v2/audit/stats')[1])
                    if stats['total_queries'] != args.prefill:
                        raise RuntimeError('prefill did not fill the configured audit ring')
                ev = events(args.scenario, identity['warmup'])
                if args.scenario == 'reload':
                    block = {'values': [line.strip() for line in (env.root / 'rule/blocklist.txt').read_text().splitlines() if line.strip() and not line.startswith('#')]}
                    ev = [dict(at=identity['warmup']+t, action='http', method='POST', path='/plugins/blocklist/post', body=block)
                          for t in [20,60,100,140,180,220]]
                if args.smoke or args.diagnostic:
                    ev = []
                ev += [dict(at=identity['warmup'], action='snapshot', pid=env.host.pid, reset_hwm=True)]
                spec = dict(corpus=str(corpus_path), oracle=str(oracle), mode='weighted', seed=501+pair,
                            rate=args.rate, duration=identity['warmup']+identity['duration'], events=ev)
                rows, samples, start = launch_sender(env, spec, output)
                window = analyze.window(
                    [dict(r, planned=r['planned']-start, sent=None if r['sent'] is None else r['sent']-start,
                          finished=None if r['finished'] is None else r['finished']-start) for r in rows], identity['warmup'], spec['duration'])
                chosen = [s for s in samples if identity['warmup'] <= s['time']-start < spec['duration'] and not s['processes']['sut'].get('missing')]
                states = [s['processes']['sut'] for s in chosen]
                correct = window['statuses'].get('correct', 0)
                cpu = (states[-1]['cpu_s']-states[0]['cpu_s']) if len(states)>1 else None
                recorded = json.loads((output / 'load/events.json').read_text())
                summary = dict(pair=pair+1, role=role, scenario=args.scenario, dns=window, prefill_dns=prefill_dns,
                               rss_median_kib=statistics.median(s['rss_kib'] for s in states),
                               cpu_us_per_correct=None if not correct or cpu is None else cpu*1e6/correct,
                               before=before, after=point(env.host.pid),
                               audit_status=json.loads(env.api('GET','/api/v2/audit/stats')[1]),
                               event_count=len(recorded), expected_event_count=len(ev), event_failures=[e for e in recorded if event_failed(e)])
                summaries.append(summary)
                save(output / 'summary.json', summary)
                save(args.output / 'summaries.json', summaries)
                import gzip
                for raw in output.rglob('requests.jsonl'):
                    with raw.open('rb') as source, gzip.open(str(raw)+'.gz', 'wb') as dest:
                        import shutil
                        shutil.copyfileobj(source, dest)
                    raw.unlink()


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--work', required=True, type=pathlib.Path)
    p.add_argument('--output', required=True, type=pathlib.Path)
    p.add_argument('--binary', action='append', required=True, help='role=/absolute/binary')
    p.add_argument('--oracle', type=pathlib.Path)
    p.add_argument('--top', type=pathlib.Path, help='explicit frozen weighted traffic input')
    p.add_argument('--oracle-only', action='store_true')
    p.add_argument('--smoke', action='store_true')
    p.add_argument('--diagnostic', action='store_true', help='single instrumented prefill probe, excluded from acceptance timing')
    p.add_argument('--rate', type=int, default=500)
    p.add_argument('--prefill', type=int, help='defaults to the frozen audit capacity')
    p.add_argument('--scenario', choices=['steady','settings','groups','reads','reload'], default='steady')
    run(p.parse_args())
