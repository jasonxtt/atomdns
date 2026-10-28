#!/usr/bin/env python3
"""Instrument a NEW disposable source copy for retained-heap accounting.

Uses the established field/capacity accounting probe; these executables must
never supply CPU, latency or RSS acceptance evidence.
"""
import argparse
import hashlib
import json
import pathlib
import shutil


def create(source, output, synthetic=False):
    if output.exists():
        raise ValueError('output must be new')
    shutil.copytree(source, output, ignore=shutil.ignore_patterns('target','.git','node_modules'))
    observer = output / 'rust/native-host/src/observer.rs'
    text = observer.read_text()
    probe = (source / 'tools/5d-1/audit_memory_compact.rs').read_text()
    if 'audit_timings:' not in text:
        probe = probe.replace('        + state.audit_timings.capacity() * size_of::<super::AuditTimingSnapshot>()\n', '')
    path = output / 'rust/native-host/src/observer/audit_memory_probe.rs'
    path.write_text(probe)
    retention = output / 'rust/native-host/src/observer/retention.rs'
    retained_text = retention.read_text()
    if synthetic:
        # Identical records alone do not fix weak-cache hint collisions:
        # RandomState chooses a different mapping in each process. Fix only
        # the diagnostic copy's hasher, identically in BASE and candidate.
        old = 'use std::collections::hash_map::RandomState;'
        if retained_text.count(old) != 1 or 'RandomState::new()' not in retained_text:
            raise ValueError('sharing hasher seam changed')
        retained_text = retained_text.replace(old, 'type RandomState = std::hash::BuildHasherDefault<std::collections::hash_map::DefaultHasher>;')
        retained_text = retained_text.replace('RandomState::new()', 'RandomState::default()')
    retention.write_text(retained_text + '\ninclude!("audit_memory_probe.rs");\n')
    if synthetic:
        fixture = pathlib.Path(__file__).with_name('audit_synthetic.rs')
        shutil.copy2(fixture, path.with_name('memory_synthetic_fixture.rs'))
        seam = '    fn observation(elapsed: Duration) -> TerminalObservation {'
        if text.count(seam) != 1:
            raise ValueError('synthetic fixture test seam changed')
        observer.write_text(text.replace(seam, '    include!("observer/memory_synthetic_fixture.rs");\n\n'+seam))
        manifest = {
            'excluded_from_timing_acceptance': True,
            'sharing_hash': 'fixed diagnostic DefaultHasher',
            'fixture_sha256': hashlib.sha256(fixture.read_bytes()).hexdigest(),
            'source_files_sha256': {str(p.relative_to(source)): hashlib.sha256(p.read_bytes()).hexdigest() for p in (source / 'rust/native-host/src/observer.rs', source / 'rust/native-host/src/observer/retention.rs')},
        }
        (output / 'memory-diagnostic.json').write_text(json.dumps(manifest, indent=2)+'\n')
        return

    needle = 'pub(crate) fn audit_snapshot(&self) -> AuditSnapshot {\n        let state = self.lock();'
    if text.count(needle) != 1:
        raise ValueError('audit snapshot seam changed')
    observer.write_text(text.replace(needle, needle + '\n        retention::audit_memory_report(&state);'))
    examples = output / 'rust/native-host/examples'
    examples.mkdir(exist_ok=True)
    shutil.copy2(source / 'tools/5d-1/audit_memory_launcher.rs', examples / 'audit_memory.rs')


if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source', required=True, type=pathlib.Path)
    p.add_argument('--output', required=True, type=pathlib.Path)
    p.add_argument('--synthetic', action='store_true', help='deterministic in-process record replay, fixed diagnostic sharing hash')
    a=p.parse_args()
    create(a.source.resolve(),a.output.resolve(),a.synthetic)
