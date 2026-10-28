#!/usr/bin/env python3
"""Derive a private two-CPU copy of the frozen offline observation tools.

SUT stays on CPU0. CoreDNS, proxy, sender and nice+10 sampler share CPU1.
The original tools and product configuration are never edited. Full protocol
readiness runs before SUT startup; every derived source hash is recorded.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil


def derive(source, destination, prioritize_sender=False):
    if not {0, 1}.issubset(os.sched_getaffinity(0)):
        raise ValueError('profile needs CPUs 0 and 1')
    if destination.exists():
        raise ValueError('derived tools destination already exists')
    replacements = {
        'test-upstream/service.py': [("'taskset','-c','2'", "'taskset','-c','1'")],
        'test-upstream/case.py': [
            ('self.mock=self.spawn(2,', 'self.mock=self.spawn(1,'),
            ("command(['python3',HERE/'service.py','reset','--root',self.upstream_root])",
             "command(['python3',HERE/'service.py','reset','--root',self.upstream_root])\n"
             "            command(['python3',HERE/'check.py','--root',self.upstream_root,'--output',self.output/'upstream-readiness'])"),
        ],
        'test-upstream/performance.py': [
            ('sampler_cpu=3,', 'sampler_cpu=1,'),
            ("files=[HERE/'performance.py',", "files=[HERE/'check.py',HERE/'performance.py',"),
            ("sys.path.insert(0,str(HERE.parent/'memory-validation'))", "sys.path.insert(0,str(HERE.parent/'memory-validation'))\nfrom lossless_post_reads import capture as capture_post_reads"),
            ("HERE.parent/'memory-validation/real_metrics.py'", "HERE.parent/'memory-validation/lossless_post_reads.py',HERE.parent/'memory-validation/real_metrics.py'"),
            ("(output/'udp-sockets.txt').write_text(pathlib.Path(f'/proc/{env.host.pid}/net/udp').read_text())", "(output/'udp-sockets.txt').write_text(pathlib.Path(f'/proc/{env.host.pid}/net/udp').read_text())\n                    capture_post_reads(env)"),
        ],
    }
    if prioritize_sender:
        replacements['test-upstream/case.py'].append((
            '    def __enter__(self):',
            "    def spawn(self, cpu, argv, name):\n"
            "        if name == 'load.log':\n"
            "            argv = ['nice', '-n', '-5', *argv]\n"
            "        return super().spawn(cpu, argv, name)\n\n"
            '    def __enter__(self):'))
        # The sampler inherits its parent's nice value. Keep its absolute
        # priority at +10 when the load sender starts at -5.
        replacements['5d-2/fload.py'] = [(
            'nice = os.nice(10)',
            'nice = os.nice(10 - os.getpriority(os.PRIO_PROCESS, 0))')]
    patched = {}
    for name, changes in replacements.items():
        original = (source / name).read_text()
        text = original
        for before, after in changes:
            if text.count(before) != 1:
                raise ValueError(f'expected one frozen source marker in {name}')
            text = text.replace(before, after)
        patched[name] = text
    shutil.copytree(source, destination, ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    for name, text in patched.items():
        (destination / name).write_text(text)
    sha = lambda path: hashlib.sha256(path.read_bytes()).hexdigest()
    receipt = {
        'profile': 'two-CPU observations, SUT0, all auxiliary processes1',
        'changes': replacements,
        'original_sha256': {name: sha(source / name) for name in replacements},
        'derived_sha256': {str(p.relative_to(destination)): sha(p) for p in destination.rglob('*.py')},
        'protocol_readiness': 'all configured fixture endpoint protocols before SUT startup',
        'post_load_reads': 'three kernel-HWM-bracketed reads per rank/cache-show/page/dump, outside DNS timing',
        'scope': 'test-only profile, no product code or default method changes',
        'nice': {'sender': -5 if prioritize_sender else 0, 'sampler': 10,
                 'sut': 0, 'upstream': 0, 'proxy': 0},
    }
    (destination.parent / 'two-cpu-method.json').write_text(json.dumps(receipt, indent=2) + '\n')
    return receipt


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--destination', type=Path, required=True)
    parser.add_argument('--prioritize-sender', action='store_true',
                        help='owned load sender nice=-5; sampler remains absolute nice=10')
    args = parser.parse_args()
    print(json.dumps(derive(args.source.resolve(), args.destination.resolve(), args.prioritize_sender)))
