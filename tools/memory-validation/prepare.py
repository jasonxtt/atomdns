#!/usr/bin/env python3
"""Prepare the current native configuration in a new private directory.

Input is a complete config root, never modified. Only the private copy is
rewired to the existing e2e-1 deterministic fixtures. No Go migration or old
switch/settings conversion is applied. Run on Linux with PyYAML available.
"""
import argparse
import hashlib
import json
import pathlib
import shutil
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / 'e2e-1'))
from prepare import definitions, mapped  # noqa: E402
from mock import GROUPS  # noqa: E402
import yaml  # noqa: E402


def prepare(source, output, audit_capacity=100000):
    if output.exists():
        raise ValueError('output must not exist')
    output.mkdir(mode=0o700, parents=True)
    root = output / 'native'
    shutil.copytree(source, root)
    for p in list(root.rglob('.DS_Store')) + list(root.rglob('._*')):
        p.unlink()
    (root / 'cache').mkdir(exist_ok=True)
    for p in (root / 'cache').glob('*.dump'):
        p.unlink()
    (root / 'webinfo').mkdir(exist_ok=True)
    settings_path = root / 'webinfo/settings.json'
    settings = json.loads(settings_path.read_text()) if settings_path.exists() else {'version': 1}
    settings.setdefault('proxies', {}).setdefault('main_proxy', {})['addr'] = '127.0.0.1:17891'
    groups = {d['tag']: d['args'] for d in definitions(root) if d['type'] == 'aliapi'}
    if set(groups) - set(GROUPS):
        raise ValueError('unmapped upstream groups')
    for tag, args in groups.items():
        entries = settings.get('upstreams', {}).get(tag, args.get('upstreams', []))
        if not any(e.get('enabled', True) for e in entries):
            entries = [{'tag': tag + '-fixture', 'addr': 'udp://127.0.0.1:9',
                        'protocol': 'udp', 'enabled': True}]
        settings.setdefault('upstreams', {})[tag] = [dict(mapped(e, tag), enabled=True) if e.get('enabled', True) else e for e in entries]
    settings_path.write_text(json.dumps(settings, ensure_ascii=False, indent=2))
    for name, value in [('audit_settings.json', {'capacity': audit_capacity}), ('webui_port_settings.json', {'port': 9099})]:
        (root / 'webinfo' / name).write_text(json.dumps(value))
    main = root / 'config_custom.yaml'
    config = yaml.safe_load(main.read_text())
    config['log'] = {'level': 'error'}
    # Keep every configured sequence/listener/rule. Egress is disabled by the
    # namespace in the harness; background jobs cannot contact the Internet.
    main.write_text(yaml.safe_dump(config, allow_unicode=True, sort_keys=False))
    # Same certificate setup used by the earlier single-core harness.
    sys.path.insert(0, str(HERE.parent / '5d-1'))
    from run import cert
    cert(root)
    (root / 'mock-delays.json').write_text(json.dumps(dict(domestic=10, foreign=80, foreignecs=80, cnfake=1, nocnfake=1)))
    fingerprint = lambda base: {str(p.relative_to(base)): hashlib.sha256(p.read_bytes()).hexdigest()
                              for p in sorted(base.rglob('*')) if p.is_file()}
    # Private: contains path names, never config contents or credentials.
    (output / 'input-manifest.json').write_text(json.dumps({'source': fingerprint(source), 'prepared': fingerprint(root),
        'runtime_features': {'route_learner': any(d['type'] == 'route_learner' for d in definitions(root))}}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=pathlib.Path, required=True)
    parser.add_argument('--output', type=pathlib.Path, required=True)
    parser.add_argument('--audit-capacity', type=int, choices=[0,100000,400000], default=100000)
    args = parser.parse_args()
    prepare(args.source.resolve(), args.output.resolve(), args.audit_capacity)
