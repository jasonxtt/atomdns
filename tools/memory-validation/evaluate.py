#!/usr/bin/env python3
"""Evaluate ONE frozen three-pair scenario; never claims whole-project acceptance."""
import argparse
import json
import pathlib
import statistics


def event_failed(event):
    result = event.get('result')
    if not isinstance(result, dict):
        return True
    if event.get('action') == 'http':
        return not 200 <= result.get('status', 0) < 300
    if event.get('action') == 'snapshot':
        return any(not isinstance(result.get(k), int) or result[k] < 0 for k in ['rss_kib','hwm_kib'])
    return True


def valid_dns_outcomes(dns, allow_empty=False):
    count = dns.get('planned', 0)
    return (isinstance(count, int) and (count > 0 or allow_empty and count == 0)
            and dns.get('sent') == count and dns.get('healthy') == count
            and dns.get('statuses') == ({'correct': count} if count else {}))


def static_oracle_block_reason(manifest):
    learner = manifest.get('runtime_features', {}).get('route_learner')
    if learner is True:
        return 'static oracle cannot model route_learner expiry or in-run learning'
    if learner is not False:
        return 'prepared manifest does not establish route_learner absence; re-prepare inputs'
    return None


def evaluate(identity, rows, cleanup):
    roles = list(identity['binaries'])
    reasons = []
    scope = dict(top_source=identity.get("top_source", "unknown"), supplementary=identity.get("top_source") != "default")
    if scope["top_source"] not in {"default", "explicit"}:
        reasons.append("missing corpus provenance")
    if identity.get('excluded_from_acceptance'):
        reasons.append('smoke/diagnostic is excluded from acceptance')
    if len(set(identity['binaries'].values())) != len(roles):
        reasons.append('baseline/candidate artifacts are byte-identical')
    if len(roles) != 2 or roles[0] != 'base':
        reasons.append('expected exactly baseline/candidate')
    if identity.get('warmup') != 60 or identity.get('duration') != 300:
        reasons.append('timing differs from frozen 60s/300s schedule')
    for role in roles:
        if sorted(r['pair'] for r in rows if r['role'] == role) != [1, 2, 3]:
            reasons.append(f'{role}: missing/duplicate pair')
    if identity.get('prefill') != identity.get('audit_capacity') or identity.get('audit_capacity') is None:
        reasons.append('missing/mismatched frozen capacity and prefill')
    for row in rows:
        if row.get('audit_status',{}).get('total_queries') != identity.get('audit_capacity'):
            reasons.append('audit ring did not remain full')
        if not valid_dns_outcomes(row.get('prefill_dns', {}), allow_empty=identity.get('prefill') == 0):
            reasons.append(f"{row['role']}/r{row['pair']}: incomplete or incorrect prefill DNS outcomes")
        dns = row['dns']
        if not valid_dns_outcomes(dns):
            reasons.append(f"{row['role']}/r{row['pair']}: incomplete or incorrect DNS outcomes")
        if row.get('expected_event_count') is None or row.get('event_count') != row['expected_event_count']:
            reasons.append('missing/incomplete scheduled events')
        if row.get('event_failures'):
            reasons.append(f"{row['role']}/r{row['pair']}: management operation failed")
        if row.get('cpu_us_per_correct') is None or row['cpu_us_per_correct'] <= 0:
            reasons.append('missing CPU evidence')
        if any(dns['correct_latency_ms'].get(p) is None for p in ['p95','p99']):
            reasons.append('missing latency evidence')
        receipt = cleanup.get(f"r{row['pair']}-{row['role']}", {})
        if not all(receipt.get(k) is True for k in ['all_owned_processes_exited','namespace_deleted','original_service_unchanged','existing_services_unchanged']):
            reasons.append('missing/failed cleanup or service identity receipt')
    if reasons:
        return dict(status='incomplete', reasons=reasons, whole_project_accepted=False, **scope)
    base = [r for r in rows if r['role'] == 'base']
    candidate = [r for r in rows if r['role'] == roles[1]]
    def metric(name, fn, maximum=False, limit=1.05):
        reducer = max if maximum else statistics.median
        old, new = reducer(fn(r) for r in base), reducer(fn(r) for r in candidate)
        return dict(name=name, baseline=old, candidate=new, ratio=new/old if old else None,
                    pass_gate=(new <= old*limit))
    gates = [metric('cpu_us_per_correct',lambda r:r['cpu_us_per_correct']),
             *[metric(p,lambda r,p=p:r['dns']['correct_latency_ms'][p]) for p in ['p95','p99']],
             metric('startup_peak_kib',lambda r:r['before']['hwm_kib'],maximum=True,limit=1),
             metric('measurement_peak_kib',lambda r:r['after']['hwm_kib'],maximum=True,limit=1),
             metric('steady_rss_kib',lambda r:r['rss_median_kib'],limit=1)]
    gates[-1]['pass_gate'] = gates[-1]['candidate'] < gates[-1]['baseline']
    return dict(status='pass' if all(g['pass_gate'] for g in gates) else 'fail', gates=gates, **scope,
                whole_project_accepted=False,
                note='This verdict covers this paired scenario only, not heap gates, other scenarios, review or soak.')


if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('root',type=pathlib.Path)
    a=p.parse_args()
    identity=json.loads((a.root/'identity.json').read_text())
    rows=json.loads((a.root/'summaries.json').read_text()) if (a.root/'summaries.json').exists() else []
    receipts={f.parent.name:json.loads(f.read_text()) for f in a.root.glob('r*-*/cleanup.json')}
    for name, receipt in receipts.items():
        guard = a.root/name/'existing-service-guard.json'
        receipt['existing_services_unchanged'] = json.loads(guard.read_text()).get('unchanged') if guard.exists() else None
    print(json.dumps(evaluate(identity,rows,receipts),indent=2))
