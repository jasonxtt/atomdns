"""Aggregate a frozen three-pair live-upstream observation, not formal acceptance."""
import statistics


def summarize_pairs(reports):
    reasons=[]
    if len(reports)!=3:reasons.append('requires three complete pairs')
    for i,report in enumerate(reports):
        try:
            expected_order=['candidate','base'] if i==1 else ['base','candidate']
            if report['role_order']!=expected_order:reasons.append('role order differs')
            if report['timed_spec']!=dict(warmup=60,duration=300,rate=500,seed=501+i,idle=30):reasons.append('frozen schedule differs')
            receipt=report['cleanup']
            if receipt.get('all_owned_parents_exited') is not True or receipt.get('existing_service_identity_unchanged') is not True or receipt.get('owned_live_group_members')!=[]:reasons.append('cleanup incomplete')
            if report['binaries']['base']==report['binaries']['candidate']:reasons.append('identical artifacts')
            for role in ['base','candidate']:
                row=report['roles'][role];timed=row['timed']
                if any(row[k]!=0 for k in ['worker_exit','fill_exit','load_exit']):reasons.append('worker incomplete')
                if row['fill_summary']['received']!=100000 or row['fill_summary']['planned']!=100000:reasons.append('prefill incomplete')
                if any(row[k]['total_queries']!=100000 for k in ['pre_load_stats','audit_stats']):reasons.append('ring not full')
                if timed['planned']!=150000 or timed['raw_rows']!=180000 or sum(timed['statuses'].values())!=150000:reasons.append('load records incomplete')
                if len(row['load_events'])!=1 or row['load_events'][0]['action']!='snapshot' or row['load_events'][0]['result']['hwm_kib']<=0:reasons.append('peak event incomplete')
            if report['roles']['base']['timed']['sequence_sha256']!=report['roles']['candidate']['timed']['sequence_sha256']:reasons.append('paired input mismatch')
            for key in ['binaries','weighted_corpus_sha256','effective_upstream_sha256','tools_sha256']:
                if report[key]!=reports[0][key]:reasons.append('identity changed: '+key)
        except (KeyError,TypeError,IndexError):
            reasons.append('malformed pair report')
    result=dict(status='incomplete' if reasons else 'observations_complete',reasons=reasons,
                correctness_checked=False,whole_project_accepted=False,timing_eligible=False)
    if reasons:return result
    try:
        metrics={}
        getters={'rss_median_kib':lambda r:r['timed']['rss_median_kib'],
                 'cpu_us_per_protocol_reply':lambda r:r['timed']['cpu_us_per_protocol_reply'],
                 'p95_ms':lambda r:r['timed']['reply_latency_ms']['p95'],
                 'p99_ms':lambda r:r['timed']['reply_latency_ms']['p99'],
                 'idle_rss_kib':lambda r:r['after_idle']['rss_kib']}
        for key,getter in getters.items():
            base=[getter(r['roles']['base']) for r in reports]
            candidate=[getter(r['roles']['candidate']) for r in reports]
            if any(v is None or v<=0 for v in base+candidate):raise ValueError('missing positive metric: '+key)
            old,new=statistics.median(base),statistics.median(candidate)
            metrics[key]=dict(base_runs=base,candidate_runs=candidate,base_median=old,candidate_median=new,
                              candidate_over_base=new/old,change_percent=(new/old-1)*100)
        result['metrics']=metrics
        result['outcomes']={role:[{k:r['roles'][role]['timed'][k] for k in ['planned','sent','protocol_replies','statuses']} for r in reports] for role in ['base','candidate']}
        result['peak_hwm_kib']={role:max([v['hwm_kib'] for r in reports for v in r['roles'][role].values() if isinstance(v,dict) and 'hwm_kib' in v]+[r['roles'][role]['load_events'][0]['result']['hwm_kib'] for r in reports]) for role in ['base','candidate']}
    except (KeyError,TypeError,ValueError) as error:
        result.update(status='incomplete',reasons=[str(error)])
    return result


if __name__=='__main__':
    import argparse
    import json
    import pathlib
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reports',type=pathlib.Path,nargs=3)
    args=parser.parse_args()
    print(json.dumps(summarize_pairs([json.loads(p.read_text()) for p in args.reports]),indent=2))
