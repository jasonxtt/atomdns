#!/usr/bin/env python3
"""Compile a private allocator probe against one explicit Rust source tree.

This diagnostic is separate from standard timing binaries. --shared selects
the candidate's new core snapshot API; BASE uses the existing owned API.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import re


def lock_packages(path):
    packages = []
    for block in path.read_text().split('[[package]]')[1:]:
        fields = {}
        for line in block.splitlines():
            match = re.fullmatch(r'(name|version|source|checksum) = (".*")', line)
            if match:
                fields[match[1]] = json.loads(match[2])
            elif re.match(r'^(name|version|source|checksum)\s*=', line):
                raise ValueError('unsupported generated Cargo lock field')
        if 'name' not in fields or 'version' not in fields:
            raise ValueError('invalid generated Cargo lock package')
        packages.append(fields)
    return packages


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--shared', action='store_true')
    args = parser.parse_args()
    source = args.source.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    def source_identity():
        return {str(p.relative_to(source)): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in sorted((source / 'rust').rglob('*'))
                if p.is_file() and 'target' not in p.relative_to(source).parts}
    frozen_source = source_identity()
    shutil.copyfile(Path(__file__).with_name('lossless-probe.rs'), output / 'main.rs')
    deps = {name: {'path': str(source / 'rust' / directory)} for name, directory in
            [('mosdns-native-host', 'native-host'), ('mosdns-cache-core', 'cache-core')]}
    manifest = ['[package]', 'name="lossless-allocation-probe"', 'version="0.0.0"',
                'edition="2024"', '[features]', 'shared=[]', '[[bin]]',
                'name="probe"', 'path="main.rs"', '[dependencies]']
    manifest += [f'{name}={{path={json.dumps(value["path"])}}}' for name, value in deps.items()]
    (output / 'Cargo.toml').write_text('\n'.join(manifest) + '\n')
    shutil.copyfile(source / 'rust/Cargo.lock', output / 'Cargo.lock')
    env = dict(os.environ, CARGO_TARGET_DIR=str(output / 'target'), CARGO_INCREMENTAL='0')
    command = ['cargo', 'build', '--offline', '--release', '--manifest-path', str(output / 'Cargo.toml')]
    if args.shared:
        command += ['--features', 'shared']
    with (output / 'build.log').open('w') as log:
        subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    if source_identity() != frozen_source:
        raise ValueError('Rust sources changed while building the diagnostic')
    original = lock_packages(source / 'rust/Cargo.lock')
    expected = {(p['name'], p['version'], p.get('source')): p.get('checksum') for p in original}
    for package in lock_packages(output / 'Cargo.lock'):
        if package['name'] == 'lossless-allocation-probe':
            continue
        key = package['name'], package['version'], package.get('source')
        if key not in expected or package.get('checksum') != expected[key]:
            raise ValueError(f'probe dependency differs from source lock: {key}')
    result = subprocess.run([str(output / 'target/release/probe')], capture_output=True, text=True, check=True)
    rows = [json.loads(line) for line in result.stdout.splitlines()]
    (output / 'measurements.json').write_text(json.dumps(rows, indent=2) + '\n')
    (output / 'identity.json').write_text(json.dumps({
        'source': str(source), 'shared': args.shared,
        'probe_sha256': hashlib.sha256((output / 'main.rs').read_bytes()).hexdigest(),
        'binary_sha256': hashlib.sha256((output / 'target/release/probe').read_bytes()).hexdigest(),
        'source_lock_sha256': hashlib.sha256((source / 'rust/Cargo.lock').read_bytes()).hexdigest(),
        'source_sha256': frozen_source,
        'rustc': subprocess.check_output(['rustc', '-vV'], text=True),
        'scope': 'requested allocation diagnostic; not RSS, retained heap or DNS timing acceptance',
    }, indent=2) + '\n')
    print(json.dumps(rows))


if __name__ == '__main__':
    main()
