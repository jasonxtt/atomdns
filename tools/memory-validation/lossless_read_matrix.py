#!/usr/bin/env python3
"""Three isolated paired HTTP-read observations after closed-loop ring fill.

No DNS timing/CPU acceptance: every fill reply, including known failures, stays
in private evidence. Independent of the timed sender's dispatch qualification.
"""
import argparse
import json
from pathlib import Path
import re
import sys

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'test-upstream'))
from case import Case
from fixture import save, sha
from prepare_case import prepare
from service import ROOT, verify_fixture
from performance import protected_changes
from load import corpus
from lossless_post_reads import capture


def run(args):
    if not re.fullmatch('[0-9a-f]{40}', args.method_head):
        raise ValueError('full method commit required')
    args.output.mkdir(mode=0o700, parents=True)
    folder, fixture = verify_fixture(args.upstream_root)
    work = args.output / 'work'
    work.mkdir()
    save(work / 'fixture-manifest.json', fixture)
    manifest = prepare(args.source, work / 'native', fixture, args.upstream_root / 'certs', True)
    save(work / 'input-manifest.json', manifest)
    questions = [q for q in corpus((work / 'native/gen/top_domains.txt').read_text())
                 if q['kind'] == 'production']
    if not questions:
        raise ValueError('full production corpus required')
    sequence = args.output / 'fill-sequence.json'
    save(sequence, questions)
    files = [HERE / 'lossless_read_matrix.py', HERE / 'lossless_post_reads.py',
             HERE / 'real_config_probe.py', HERE.parent / 'test-upstream' / 'case.py',
             HERE.parent / 'test-upstream' / 'prepare_case.py',
             HERE.parent / 'test-upstream' / 'performance.py',
             HERE.parent / 'test-upstream' / 'check.py',
             HERE.parent / 'test-upstream' / 'service.py',
             HERE.parent / 'test-upstream' / 'fixture.py',
             HERE.parent / 'test-upstream' / 'proxy.py',
             HERE.parent / '5d-2' / 'harness.py', HERE.parent / '5d-1' / 'load.py',
             HERE.parent / 'e2e-1' / 'dnswire.py', HERE / 'run.py']
    def identity():
        return dict(base=sha(args.base), candidate=sha(args.candidate),
                    input=sha(work / 'input-manifest.json'), sequence=sha(sequence),
                    fixture=sha(folder / 'manifest.json'),
                    tools={str(p.relative_to(HERE.parent)): sha(p) for p in files})
    frozen = identity()
    if frozen['base'] == frozen['candidate']:
        raise ValueError('distinct binaries required')
    report = dict(scope='closed-loop fill plus HTTP read peaks only; no DNS CPU/latency acceptance',
                  method_head=args.method_head, identity=frozen, questions=len(questions), pairs=[])
    save(args.output / 'report.json', report)
    try:
        for index in [1, 2, 3]:
            pair = dict(pair=index, order=['candidate', 'base'] if index == 2 else ['base', 'candidate'], roles={})
            report['pairs'].append(pair)
            for role in pair['order']:
                if identity() != frozen:
                    raise ValueError('method/input/binary changed')
                save(args.output / 'progress.json', dict(pair=index, role=role, stage='startup'))
                output = args.output / f'r{index}' / role
                with Case(work, getattr(args, role), output, args.upstream_root) as env:
                    spec = dict(sequence=str(sequence), count=100000,
                                raw=str(output / 'fill-dns.jsonl.gz'), summary=str(output / 'fill-summary.json'))
                    save(output / 'fill-spec.json', spec)
                    save(args.output / 'progress.json', dict(pair=index, role=role, stage='fill100000'))
                    child = env.spawn(1, ['python3', HERE / 'real_config_probe.py', '--fill-worker', output / 'fill-spec.json'], 'fill.log')
                    if child.wait() != 0:
                        raise RuntimeError('closed-loop fill failed')
                    fill = json.loads((output / 'fill-summary.json').read_text())
                    status, text, _ = env.api('GET', '/api/v2/audit/stats')
                    if status != 200 or json.loads(text)['total_queries'] != 100000 or fill['planned'] != 100000 or fill['received'] != 100000:
                        raise ValueError('incomplete ring fill')
                    save(args.output / 'progress.json', dict(pair=index, role=role, stage='reads75'))
                    capture(env)
                    changes = protected_changes(work / 'native', env.root, manifest)
                    if not changes['ok']:
                        raise ValueError('protected config changed')
                cleanup = json.loads((output / 'cleanup.json').read_text())
                guard = json.loads((output / 'existing-service-guard.json').read_text())
                reads = json.loads((output / 'post-reads/report.json').read_text())
                if not all(cleanup.values()) or not guard['unchanged'] or not reads.get('all_passed') or len(reads['rows']) != 75:
                    raise ValueError('incomplete reads/cleanup')
                pair['roles'][role] = dict(fill=fill, ring=100000, changes=changes,
                                          cleanup=cleanup, guard=guard, reads=reads)
                save(args.output / 'report.json', report)
                print(index, role, '100000 replies and 75 reads complete', flush=True)
        report['observations_complete'] = True
        save(args.output / 'progress.json', dict(stage='completed'))
    except Exception as error:
        report['error'] = f'{type(error).__name__}: {error}'
        save(args.output / 'progress.json', dict(stage='failed', error=report['error']))
        raise
    finally:
        save(args.output / 'report.json', report)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['source', 'base', 'candidate', 'output']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--upstream-root', type=Path, default=ROOT)
    parser.add_argument('--method-head', required=True)
    run(parser.parse_args())
