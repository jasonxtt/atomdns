"""Live DNS observation metrics. Never asserts answer correctness or acceptance."""
import collections
import gzip
import hashlib
import json
import math
import statistics


def observed_status(value, name, qtype):
    if value['questions'] != [(name+'.',qtype,1)]:
        return 'observed_wrong_question'
    if any(not 0<=rr[4]<2**32 for sec in ['answers','authority','additional'] for rr in value[sec]):
        return 'observed_invalid_ttl'
    return {0:'observed_noerror',2:'observed_servfail'}.get(value['rcode'],'observed_other_rcode')


def summarize(rows, samples, start, duration):
    chosen=[r for r in rows if start<=r['planned']<start+duration]
    replies=[r for r in chosen if r['status'] in {'observed_noerror','observed_servfail','observed_other_rcode'} and r['finished'] is not None]
    latencies=sorted((r['finished']-r['planned'])*1000 for r in replies)
    states=[(s['time'],s['processes']['sut']) for s in samples if start<=s['time']<=start+duration and not s['processes']['sut'].get('missing')]
    span=states[-1][0]-states[0][0] if len(states)>1 else None
    cpu=states[-1][1]['cpu_s']-states[0][1]['cpu_s'] if len(states)>1 else None
    def percentile(p):
        return latencies[math.ceil(len(latencies)*p)-1] if latencies else None
    return dict(planned=len(chosen),sent=sum(r['sent'] is not None for r in chosen),
                statuses=dict(collections.Counter(r['status'] for r in chosen)),protocol_replies=len(replies),
                on_time_500ms=sum(r['finished']-r['planned']<=.5 for r in replies),
                reply_latency_ms=dict(p95=percentile(.95),p99=percentile(.99)),
                cpu_sample_span_s=span,cpu_seconds=cpu,
                cpu_us_per_protocol_reply=cpu*1e6/len(replies) if cpu is not None and replies else None,
                rss_median_kib=statistics.median(s['rss_kib'] for _,s in states) if states else None,
                rss_min_kib=min(s['rss_kib'] for _,s in states) if states else None,
                rss_max_kib=max(s['rss_kib'] for _,s in states) if states else None,
                correctness_checked=False,whole_project_accepted=False,timing_eligible=False,
                denominator='all well-formed question-matched replies, including SERVFAIL; not correct DNS queries')


def read_summary(root, warmup, duration):
    meta=json.loads((root/'summary.json').read_text())
    requests=root/'requests.jsonl.gz'
    with gzip.open(requests,'rt') as source:rows=[json.loads(line) for line in source]
    samples=json.loads((root/'resources.json').read_text())
    result=summarize(rows,samples,meta['start']+warmup,duration)
    result.update(sequence_sha256=hashlib.sha256(json.dumps([r['index'] for r in rows]).encode()).hexdigest(),
                  raw_rows=len(rows),orphan_packets=meta['orphan_packets'])
    return result
