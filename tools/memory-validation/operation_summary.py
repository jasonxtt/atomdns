"""Pure gates for the user-approved operation-peak protocol."""
import collections
import json
import statistics

KNOWN_FAILURES=frozenset([3972,3973,5187])


def parse_api_response(method,status,text):
    if status not in {200,201,202,204}:raise ValueError(f'HTTP {status}')
    if method=='GET' and (status!=200 or not text.strip()):raise ValueError('GET requires a JSON 200 response')
    if not text.strip():return None
    try:return json.loads(text)
    except ValueError:
        if method=='GET':raise
        return text


def dns_outcomes(rows,ignore_unsent=False):
    statuses=collections.Counter(r['status'] for r in rows)
    known=sum(r['status']=='observed_servfail' and r['index'] in KNOWN_FAILURES for r in rows)
    raw=len(rows)-statuses['observed_noerror']-known
    ignored=statuses['unsent'] if ignore_unsent else 0
    unexpected=raw-ignored
    return dict(statuses=dict(statuses),known_servfail=known,raw_unexpected=raw,ignored_unsent=ignored,unexpected=unexpected,ok=unexpected==0,
                correctness_checked=False)


def validate_controls(prior,identity):
    if any(prior.get('identity',{}).get(k)!=identity[k] for k in ['base','candidate','input','corpus','core']):
        raise ValueError('prior M0 identity mismatch')
    try:
        for pair in ['1','2','3']:
            for role in ['base','candidate']:
                row=prior['R2']['M0'][pair][role]
                if row.get('ok') is not True or row['rss145_kib']<=0 or row['idle']['rss_kib']<=0:
                    raise ValueError('invalid prior M0 control')
    except (KeyError,TypeError) as e:raise ValueError('incomplete prior M0 controls') from e
    return True


def memory_gate(base,candidate,ratios=None,cap_kib=262144):
    if len(base)!=3 or len(candidate)!=3 or (ratios is not None and len(ratios)!=3):
        raise ValueError('three complete role pairs required')
    b=statistics.median(base);c=statistics.median(candidate)
    all_max=max(base+candidate)
    median_ok=c<=b*1.01
    cap_ok=cap_kib is None or all_max<=cap_kib
    ratio_ok=ratios is None or all(0<r<=1.2 for pair in ratios for r in pair)
    return dict(base_median_kib=b,candidate_median_kib=c,all_run_max_kib=all_max,
                median_ok=median_ok,cap_ok=cap_ok,ratio_ok=ratio_ok,ok=median_ok and cap_ok and ratio_ok)


def event_gate(events,start,operations):
    snapshots=[e for e in events if e.get('action')=='snapshot']
    requests=[e for e in events if e.get('action')=='http']
    if len(snapshots)!=2 or len(requests)!=operations or len(events)!=operations+2:return False
    before=next((e for e in snapshots if e.get('at')==35),{})
    if not before.get('reset_hwm') or not any(e.get('at')==145 for e in snapshots):return False
    for e in events:
        if abs(e['started']-start-e['at'])>1:return False
        value=e.get('result')
        if not isinstance(value,dict):return False
        if e['action']=='snapshot' and not {'rss_kib','hwm_kib'}<=value.keys():return False
        if e['action']=='http' and value.get('status') not in {200,201,202,204}:return False
    return True
