"""Three frozen single-upstream observations, explicitly not full acceptance."""
import statistics


def incomplete_send_collection_allowed(timed):
    """Collection only: never makes summarize() qualify an incomplete slot."""
    try:
        statuses=timed['statuses'];missing=150000-timed['sent']
        return (timed['planned']==150000 and timed['raw_rows']==180000
                and 0<missing<150000 and timed['protocol_replies']==timed['sent']
                and statuses.get('unsent',0)==missing and sum(statuses.values())==150000
                and set(statuses)<={'observed_noerror','observed_servfail','unsent'}
                and timed['orphan_packets']==0)
    except (KeyError,TypeError):return False


def descriptive_metrics(reports):
    """Unconditional numerical description, separate from coverage verdicts."""
    if len(reports)!=3:raise ValueError('three paired roles required for description')
    metrics={}
    getters={'rss_median_kib':lambda x:x['rss_median_kib'],'cpu_us_per_protocol_reply':lambda x:x['cpu_us_per_protocol_reply'],
             'p95_ms':lambda x:x['reply_latency_ms']['p95'],'p99_ms':lambda x:x['reply_latency_ms']['p99']}
    for name,get in getters.items():
        b=[get(r['roles']['base']['timed']) for r in reports];c=[get(r['roles']['candidate']['timed']) for r in reports]
        if any(v is None or v<=0 for v in b+c):raise ValueError('missing positive metric')
        old,new=statistics.median(b),statistics.median(c)
        metrics[name]=dict(base_runs=b,candidate_runs=c,base_median=old,candidate_median=new,change_percent=100*(new/old-1),pair_change_percent=[100*(y/x-1) for x,y in zip(b,c)])
    return metrics


def summarize(reports):
    reasons=[]
    if len(reports)!=3:reasons.append('three complete pairs required')
    for i,r in enumerate(reports,1):
        try:
            if r['pair']!=i or r['seed']!=500+i or r['order']!=(['candidate','base'] if i==2 else ['base','candidate']):reasons.append('order/seed changed')
            if r['identity']!=reports[0]['identity']:reasons.append('identity changed')
            for role in ['base','candidate']:
                row=r['roles'][role];t=row['timed'];receipt=row['cleanup']
                if any(receipt.get(k) is not True for k in ['all_owned_processes_exited','namespace_deleted','original_service_unchanged']) or row['existing_identity'] is not True:reasons.append('cleanup/identity failed')
                if row['pre_count']!=100000 or row['post_count']!=100000:reasons.append('ring not full')
                if not row['protected_ok']:reasons.append('protected config changed')
                if any(t[k]!=150000 for k in ['planned','sent','protocol_replies']) or t['raw_rows']!=180000 or sum(t['statuses'].values())!=150000:reasons.append('packet shortfall')
            if r['roles']['base']['timed']['sequence_sha256']!=r['roles']['candidate']['timed']['sequence_sha256']:reasons.append('paired sequence changed')
        except (KeyError,IndexError,TypeError):reasons.append('malformed report')
    result=dict(status='incomplete' if reasons else 'observations_complete',reasons=reasons,correctness_checked=False,whole_project_accepted=False,timing_eligible=False,
                scope='single supplier per business group, fixed CoreDNS; existing CNAME defect retained; not correctness-qualified performance')
    if reasons:return result
    try:result['metrics']=descriptive_metrics(reports)
    except ValueError:
        result.update(status='incomplete',reasons=['missing positive metric']);return result
    result['max_hwm_kib']={role:max(p['hwm_kib'] for r in reports for p in r['roles'][role]['points']) for role in ['base','candidate']}
    result['outcomes']={role:[r['roles'][role]['timed']['statuses'] for r in reports] for role in ['base','candidate']}
    return result
