#!/usr/bin/env python3
"""Decompose existing live-DNS timings without altering any original verdict."""
import argparse
import bisect
import gzip
import hashlib
import json
import math
import pathlib


def quantile(values, p):
    return sorted(values)[math.ceil(len(values)*p)-1] if values else None


def analyze(root):
    result=[]
    for pair in range(1,4):
        for role in ['base','candidate']:
            case=root/f'r{pair}'/role/'load'
            meta=json.loads((case/'summary.json').read_text());start=meta['start']+60
            samples=json.loads((case/'resources.json').read_text());times=[s['time'] for s in samples]
            with gzip.open(case/'requests.jsonl.gz','rt') as source:
                rows=[r for line in source for r in [json.loads(line)] if start<=r['planned']<start+300]
            measured=[s for s in samples if start<=s['time']<start+300];a,b=measured[0],measured[-1]
            cpu={name:b['processes'][name]['cpu_s']-a['processes'][name]['cpu_s'] for name in ['sut','sender']}
            lag=[];wire=[];near=slow=slow_near=0
            for row in rows:
                if row['sent'] is None:continue
                dispatch=1000*(row['sent']-row['planned']);lag.append(dispatch)
                if row['finished'] is not None:wire.append(1000*(row['finished']-row['sent']))
                index=bisect.bisect_left(times,row['planned'])
                close=min((abs(times[k]-row['planned']) for k in [index-1,index] if 0<=k<len(times)),default=999)<.010
                near+=close
                if dispatch>2:slow+=1;slow_near+=close
            result.append(dict(pair=pair,role=role,planned=len(rows),
                dispatch_ms={f'p{p}':quantile(lag,p/100) for p in [95,99]},
                receive_after_send_ms={f'p{p}':quantile(wire,p/100) for p in [95,99]},
                cpu_seconds=cpu,sut_over_sender_cpu=cpu['sut']/cpu['sender'],
                sample_neighborhood_fraction=near/len(lag),slow_dispatch_count=slow,
                slow_dispatch_near_sample_fraction=slow_near/slow if slow else None,
                raw_sha256={name:hashlib.sha256((case/name).read_bytes()).hexdigest() for name in ['summary.json','resources.json','requests.jsonl.gz']}))
    return dict(scope='retrospective diagnosis only; quantiles are not additive; correlation is not a causal verdict',records=result,whole_project_accepted=False)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('root',type=pathlib.Path)
    print(json.dumps(analyze(p.parse_args().root),indent=2))
