"""Read-only comparison of the completed, private read-matrix raw bodies.

Only absolute stored/expiry times are normalized to retention intervals. DNS
wire (including ID/TTL), labels and origin bytes are compared without rewriting.
Output contains counts, not query names or cached answers.
"""
import argparse
import gzip
import hashlib
import json
from pathlib import Path


def varint(data, offset):
    result = 0
    for shift in range(0, 70, 7):
        byte = data[offset]
        offset += 1
        result |= (byte & 127) << shift
        if not byte & 128:
            return result, offset
    raise ValueError('oversized varint')


def fields(data):
    result, offset = {}, 0
    while offset < len(data):
        key, offset = varint(data, offset)
        if key & 7 == 0:
            value, offset = varint(data, offset)
        elif key & 7 == 2:
            size, offset = varint(data, offset)
            value = data[offset:offset + size]
            if len(value) != size:
                raise ValueError('incomplete field')
            offset += size
        else:
            raise ValueError('unexpected wire type')
        if key >> 3 in result:
            raise ValueError('duplicate field')
        result[key >> 3] = value
    return result


def entries(body):
    data = gzip.decompress(body)
    result, offset = {}, 0
    while offset < len(data):
        if offset + 8 > len(data):
            raise ValueError('short frame')
        size = int.from_bytes(data[offset:offset + 8], 'big')
        offset += 8
        if size > 1024 * 1024 or offset + size > len(data):
            raise ValueError('invalid frame')
        block = fields(data[offset:offset + size])
        offset += size
        if set(block) != {1}:
            raise ValueError('unexpected native block')
        row = fields(block[1])
        if not {1, 2, 3, 4, 5}.issubset(row) or not set(row).issubset(range(1, 8)):
            raise ValueError('unexpected native entry')
        key = row.pop(1)
        row[3] -= row[5]
        row[4] -= row.pop(5)
        if key in result:
            raise ValueError('duplicate cache key')
        result[key] = row
    return result


def compare(root):
    report = json.loads((root / 'report.json').read_text())
    if not report.get('observations_complete') or len(report['pairs']) != 3:
        raise ValueError('three completed pairs required')
    ranks, dumps, shows = [], [], []
    for index, pair in enumerate(report['pairs'], 1):
        expected = ['candidate', 'base'] if index == 2 else ['base', 'candidate']
        if pair['pair'] != index or pair['order'] != expected:
            raise ValueError('unexpected pair/order')
        bodies = {}
        for role in ['base', 'candidate']:
            row = pair['roles'][role]
            if row['ring'] != 100000 or not all(row['cleanup'].values()) or not row['guard']['unchanged']:
                raise ValueError('incomplete ring/cleanup')
            if not row['reads'].get('all_passed') or len(row['reads']['rows']) != 75:
                raise ValueError('incomplete reads')
            for read in row['reads']['rows']:
                if Path(read['raw']).name != read['raw'] or read['status'] != 200:
                    raise ValueError('invalid body path/status')
                body = (root / f'r{index}' / role / 'post-reads' / read['raw']).read_bytes()
                if len(body) != read['bytes'] or hashlib.sha256(body).hexdigest() != read['sha256']:
                    raise ValueError('raw body identity changed')
                bodies[role, read['name'], read['repeat']] = body
        for repeat in range(3):
            for kind in ['domain', 'client', 'domain_set', 'effective']:
                name = 'rank_' + kind
                a, b = [json.loads(bodies[role, name, repeat]) for role in ['base', 'candidate']]
                ranks.append(dict(pair=index, repeat=repeat, kind=kind, rows=[len(a), len(b)], equal=a == b))
            for cache in range(7):
                for operation in ['show', 'page']:
                    name = f'cache_{operation}_{cache}'
                    def normalize(body):
                        return b'\n'.join(line.partition(b':')[0] + b':<absolute-time>'
                                          if line.partition(b':')[0] in
                                          [b'StoredTime', b'MsgExpire', b'CacheExpire'] else line
                                          for line in body.split(b'\n'))
                    a, b = [normalize(bodies[role, name, repeat]) for role in ['base', 'candidate']]
                    shows.append(dict(pair=index, repeat=repeat, operation=operation, cache=cache,
                                      equal=a == b))
                name = f'cache_dump_{cache}'
                a, b = [entries(bodies[role, name, repeat]) for role in ['base', 'candidate']]
                common = a.keys() & b.keys()
                differences = {str(field): sum(a[key].get(field) != b[key].get(field) for key in common)
                               for field in [2, 3, 4, 6, 7]}
                dumps.append(dict(pair=index, repeat=repeat, cache=cache, entries=[len(a), len(b)],
                                  key_sets_equal=a.keys() == b.keys(), field_differences=differences))
    equal = all(row['equal'] for row in ranks + shows) and all(
        row['key_sets_equal'] and not any(row['field_differences'].values()) for row in dumps)
    return dict(scope='all read-matrix repetitions; absolute wall times excluded only',
                all_equal=equal, ranks=ranks, shows=shows, dumps=dumps)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = compare(args.root)
    with args.output.open('x') as target:
        target.write(json.dumps(result, indent=2) + '\n')
    print(json.dumps(dict(all_equal=result['all_equal'], rank_comparisons=len(result['ranks']),
                          show_page_comparisons=len(result['shows']), dump_comparisons=len(result['dumps']))))
    if not result['all_equal']:
        raise SystemExit(1)
