"""Post-load HTTP read peaks, outside the frozen DNS timing window.

Only reads the owned case. Raw bodies stay in its private evidence directory.
Kernel HWM is reset before each operation; all captures/errors are retained.
"""
import hashlib
import gzip
import argparse
import json
from pathlib import Path
import time
import urllib.error
import urllib.parse
import urllib.request


def point(pid):
    values = {}
    for line in Path(f'/proc/{pid}/status').read_text().splitlines():
        key, _, value = line.partition(':')
        if key in ('VmRSS', 'VmHWM'):
            values[key] = int(value.split()[0])
    return values


def validate_body(name, body):
    if name.startswith('rank_'):
        rows = json.loads(body)
        if not isinstance(rows, list) or len(rows) > 500 or any(
                not isinstance(row, dict) or not isinstance(row.get('key'), str)
                or type(row.get('count')) is not int or row['count'] < 0 for row in rows):
            raise ValueError('invalid rank response shape')
    elif name.startswith('cache_dump_'):
        raw = gzip.decompress(body)
        offset = 0
        count = 0
        while offset < len(raw):
            if offset + 8 > len(raw):
                raise ValueError('short cache dump frame')
            size = int.from_bytes(raw[offset:offset + 8], 'big')
            offset += 8
            if size > 1024 * 1024 or offset + size > len(raw):
                raise ValueError('oversized or incomplete cache dump frame')
            offset += size
            count += 1
        if count > 100000:
            raise ValueError('too many cache dump entries')
    elif body and not body.decode('utf-8').startswith('----- Cache Entry -----\nKey:'):
        raise ValueError('invalid cache show response')


def run(pid, start_ticks, output, port):
    output.mkdir()
    base = f'http://127.0.0.1:{port}'
    with urllib.request.urlopen(base + '/api/v1/cache/inventory', timeout=30) as response:
        inventory = json.load(response)['caches']
    operations = [('rank_' + kind, '/api/v2/audit/rank/' + kind + '?limit=500')
                  for kind in ('domain', 'client', 'domain_set', 'effective')]
    for index, item in enumerate(inventory):
        tag = urllib.parse.quote(item['tag'], safe='')
        operations += [(f'cache_show_{index}', f'/plugins/{tag}/show?limit=100'),
                       (f'cache_page_{index}', f'/plugins/{tag}/show?offset=100&limit=100'),
                       (f'cache_dump_{index}', f'/plugins/{tag}/dump')]
    result = {'scope': 'post-load reads, not part of DNS CPU/latency measurements', 'rows': []}
    try:
        for name, path in operations:
            for repeat in range(3):
                current = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[19]
                if current != start_ticks:
                    raise RuntimeError('owned target PID identity changed')
                Path(f'/proc/{pid}/clear_refs').write_text('5\n')
                before = point(pid)
                began = time.monotonic()
                try:
                    with urllib.request.urlopen(base + path, timeout=60) as response:
                        status, body = response.status, response.read()
                except urllib.error.HTTPError as error:
                    status, body = error.code, error.read()
                after = point(pid)
                filename = f'{name}-{repeat}.body'
                (output / filename).write_bytes(body)
                result['rows'].append({'name': name, 'repeat': repeat, 'path': path,
                                       'before': before, 'after': after,
                                       'elapsed_s': time.monotonic() - began,
                                       'status': status, 'bytes': len(body),
                                       'sha256': hashlib.sha256(body).hexdigest(), 'raw': filename})
                (output / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
                if status != 200:
                    raise RuntimeError(f'post-load read failed: {name}: {status}')
                validate_body(name, body)
        result['all_passed'] = True
    except Exception as error:
        result['error'] = f'{type(error).__name__}: {error}'
        raise
    finally:
        (output / 'report.json').write_text(json.dumps(result, indent=2) + '\n')


def capture(env):
    if env.host.poll() is not None:
        raise RuntimeError('owned case exited before read diagnostics')
    start = Path(f'/proc/{env.host.pid}/stat').read_text().rsplit(')', 1)[1].split()[19]
    # The API binds loopback in the case namespace. Keep both the read worker
    # and its lifecycle under the existing owned-process supervisor.
    child = env.spawn(1, ['python3', Path(__file__).resolve(), '--pid', env.host.pid,
                         '--start-ticks', start, '--output', env.output / 'post-reads',
                         '--port', env.api_port], 'post-reads.log')
    if child.wait() != 0 or env.host.poll() is not None:
        raise RuntimeError('post-load read worker or owned host failed')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pid', type=int, required=True)
    parser.add_argument('--start-ticks', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--port', type=int, required=True)
    args = parser.parse_args()
    run(args.pid, args.start_ticks, args.output, args.port)
