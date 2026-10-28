#!/usr/bin/env python3
"""Decompose recorded DNS observation latency; never qualifies a failed run."""
import argparse
import collections
import gzip
import hashlib
import json
import math
from pathlib import Path


def quantiles(values):
    ordered = sorted(values)
    return {name: ordered[math.ceil(len(ordered) * fraction) - 1] if ordered else None
            for name, fraction in [('p50', .5), ('p95', .95), ('p99', .99), ('p999', .999), ('max', 1)]}


def attribute(rows, start, duration):
    """Retain failures and separate dispatch lag from observed socket round trip."""
    selected = [r for r in rows if start <= r['planned'] < start + duration]
    if len({r['seq'] for r in selected}) != len(selected):
        raise ValueError('duplicate scheduled sequence')
    replied = [r for r in selected if r['finished'] is not None]
    for row in replied:
        if row['sent'] is None or not row['planned'] <= row['sent'] <= row['finished']:
            raise ValueError('invalid reply timestamps')
    sent = [r for r in selected if r['sent'] is not None]
    if any(r['sent'] < r['planned'] for r in sent):
        raise ValueError('send precedes schedule')
    # Quantiles of separate components do not add to the total quantile.
    metrics = {name: quantiles([(r[right] - r[left]) * 1000 for r in members])
               for name, left, right, members in [
                   ('dispatch_ms', 'planned', 'sent', sent),
                   ('socket_roundtrip_ms', 'sent', 'finished', replied),
                   ('planned_to_receive_ms', 'planned', 'finished', replied)]}
    return dict(scope='offline diagnostic; socket RTT includes receiver scheduling; not server execution time',
                timing_accepted=False, planned=len(selected), sent=len(sent), replies=len(replied),
                statuses=dict(collections.Counter(r['status'] for r in selected)),
                unsent_sequences=[r['seq'] for r in selected if r['status'] == 'unsent'],
                **metrics)


def read(root, warmup=60, duration=300):
    summary = json.loads((root / 'summary.json').read_text())
    raw = root / 'requests.jsonl.gz'
    with gzip.open(raw, 'rt') as source:
        rows = [json.loads(line) for line in source]
    result = attribute(rows, summary['start'] + warmup, duration)
    result.update(raw_rows=len(rows), raw_sha256=hashlib.sha256(raw.read_bytes()).hexdigest(),
                  orphan_packets=summary['orphan_packets'])
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--load', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = read(args.load)
    with args.output.open('x') as target:
        json.dump(result, target, indent=2)
        target.write('\n')
