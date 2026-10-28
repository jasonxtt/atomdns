#!/usr/bin/env python3
"""Frozen single-upstream memory/performance observations; retain known failures.

Does not use or waive the formal functional/learner acceptance guard. All replies,
including SERVFAIL, remain visible; CPU denominator is protocol replies only.
"""
import argparse
import gzip
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import sys
import time
import urllib.request

from fixture import save,sha
from service import ROOT,NS,PEER,verify_fixture
from prepare_case import prepare
from case import Case
from performance_summary import summarize,descriptive_metrics,incomplete_send_collection_allowed
HERE=pathlib.Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'memory-validation'))
from real_config_probe import fill
from real_metrics import read_summary
from run import point
from load import corpus,semantic

SCHEDULE=dict(warmup=60,duration=300,rate=500,idle=30,fill=100000)


def protected_changes(prepared,runtime,manifest):
    import yaml
    changed=[];ok=True
    for name,old in manifest['prepared'].items():
        p=runtime/name;new=sha(p) if p.exists() else None
        if new==old:continue
        runtime_state=name in {'gen/route_learner.json','gen/top_domains.txt'}
        equal=False
        if name=='sub_config/special_groups.yaml' and p.exists():equal=yaml.safe_load((prepared/name).read_text())==yaml.safe_load(p.read_text())
        if not runtime_state and not equal:ok=False
        changed.append(dict(path=name,before=old,after=new,runtime_state=runtime_state,generated_yaml_equal=equal))
    return dict(ok=ok,checked=len(manifest['prepared']),changed=changed)


def paired_dns(base,candidate):
    prior={};seen=set();different=[];failures={}
    for role,path in [('base',base),('candidate',candidate)]:
        counts={}
        with gzip.open(path/'load/responses.jsonl.gz','rt') as source:
            for line in source:
                value=json.loads(line);seq=value['seq']
                if not 30000<=seq<180000:continue
                key=str(value['dns']['rcode']);counts[key]=counts.get(key,0)+1
                digest=hashlib.sha256(semantic(value['dns']).encode()).hexdigest()
                if role=='base':
                    if seq in prior:raise ValueError('duplicate BASE response')
                    prior[seq]=digest
                else:
                    if seq in seen:raise ValueError('duplicate candidate response')
                    seen.add(seq)
                    if prior.get(seq)!=digest:different.append(dict(seq=seq,corpus_index=value['index']))
        failures[role]=counts
    return dict(scope='per-scheduled-query DNS semantic equality; raw TTL/flags/wire retained separately; not learner/route qualification',planned=150000,
                compared=len(seen & prior.keys()),missing_candidate=len(prior.keys()-seen),extra_candidate=len(seen-prior.keys()),
                differences=len(different),first_differences=different[:30],rcode_counts=failures)


def metrics_snapshot(root,output):
    runtime=json.loads((root/'runtime.json').read_text())
    with urllib.request.urlopen('http://198.51.100.2:19153/metrics',timeout=5) as response:(output/'upstream-metrics.txt').write_bytes(response.read())
    raw=subprocess.check_output(['ip','netns','exec',NS,'tc','-s','-j','qdisc','show','dev',PEER],text=True)
    (output/'qdisc.json').write_text(raw)
    return runtime['pid']


def run(args):
    if not re.fullmatch(r'[0-9a-f]{40}',args.method_head):raise ValueError('full method commit required')
    if shutil.disk_usage(args.output.parent).free<700*1024*1024:raise ValueError('insufficient private evidence budget')
    args.output.mkdir(mode=0o700,parents=True)
    folder,fixture=verify_fixture(args.upstream_root)
    work=args.output/'work';work.mkdir();save(work/'fixture-manifest.json',fixture)
    manifest=prepare(args.source,work/'native',fixture,args.upstream_root/'certs',True);save(work/'input-manifest.json',manifest)
    settings=json.loads((work/'native/webinfo/settings.json').read_text())
    if any(len(es)!=1 or not es[0].get('enabled') for es in settings['upstreams'].values()):raise ValueError('not single supplier per group')
    rows=[v for v in corpus((work/'native/gen/top_domains.txt').read_text()) if v['kind']=='production']
    save(args.output/'corpus.json',rows);save(args.output/'fill-sequence.json',rows[:128])
    files=[HERE/'performance.py',HERE/'performance_summary.py',HERE/'prepare_case.py',HERE/'case.py',HERE/'service.py',HERE/'fixture.py',HERE/'proxy.py',HERE.parent/'5d-2/fload.py',HERE.parent/'5d-2/harness.py',HERE.parent/'5d-1/load.py',HERE.parent/'e2e-1/dnswire.py',HERE.parent/'memory-validation/real_metrics.py',HERE.parent/'memory-validation/real_config_probe.py']
    tools={str(p.relative_to(HERE.parent)):sha(p) for p in files}
    identity=dict(base=sha(args.base),candidate=sha(args.candidate),input=sha(work/'input-manifest.json'),fixture=sha(folder/'manifest.json'),corpus=sha(args.output/'corpus.json'),tools=tools,method_head=args.method_head)
    if identity['base']==identity['candidate']:raise ValueError('candidate must be a distinct artifact')
    report=dict(scope='three paired single-per-group controlled observations; known CNAME defect retained; not whole acceptance',whole_project_accepted=False,correctness_checked=False,timing_eligible=False,
                profile='single-per-group',schedule=SCHEDULE,identity=identity,questions=len(rows),pairs=[])
    report['collect_incomplete_sends']=args.collect_incomplete_sends
    save(args.output/'report.json',report)
    def progress(**value):save(args.output/'progress.json',dict(wall_time=time.time(),**value))
    try:
        for index in [1,2,3]:
            pair=dict(pair=index,order=['candidate','base'] if index==2 else ['base','candidate'],seed=500+index,identity=identity,roles={})
            pair_root=args.output/f'r{index}';pair_root.mkdir()
            for role in pair['order']:
                if tools!={str(p.relative_to(HERE.parent)):sha(p) for p in files}:raise ValueError('method source changed in batch')
                output=pair_root/role;binary=args.base if role=='base' else args.candidate
                if sha(binary)!=identity[role] or sha(work/'input-manifest.json')!=identity['input'] or sha(args.output/'corpus.json')!=identity['corpus'] or sha(folder/'manifest.json')!=identity['fixture']:
                    raise ValueError('binary/input/fixture identity changed in batch')
                progress(pair=index,role=role,stage='startup')
                row=dict(points=[])
                with Case(work,binary,output,args.upstream_root) as env:
                    row['points'].append(point(env.host.pid))
                    progress(pair=index,role=role,stage='prefill100000')
                    fill_spec=dict(sequence=str(args.output/'fill-sequence.json'),count=100000,raw=str(output/'fill-dns.jsonl.gz'),summary=str(output/'fill-summary.json'))
                    save(output/'fill-spec.json',fill_spec)
                    sender=env.spawn(1,['python3',HERE.parent/'memory-validation/real_config_probe.py','--fill-worker',output/'fill-spec.json'],'fill.log')
                    if sender.wait()!=0:raise RuntimeError('prefill failed')
                    row['fill']=json.loads((output/'fill-summary.json').read_text())
                    if row['fill']['planned']!=100000 or row['fill']['received']!=100000:raise ValueError('prefill shortfall')
                    _,text,_=env.api('GET','/api/v2/audit/stats');row['pre_count']=json.loads(text)['total_queries']
                    if row['pre_count']!=100000:raise ValueError('ring not full')
                    row['points'].append(point(env.host.pid))
                    before=output/'upstream-before';before.mkdir();upstream_pid=metrics_snapshot(args.upstream_root,before)
                    spec=dict(corpus=str(args.output/'corpus.json'),mode='weighted',rate=500,duration=360,seed=500+index,deadline_s=5,
                              observation_only=True,sampling_process=True,sampler_cpu=3,extra_pids={'proxy':env.mock.pid},precise_timer=True,retain_wire=True,
                              pid=env.host.pid,mock_pid=upstream_pid,root=str(env.root),api_port=49099,dns_port=40053,output=str(output/'load'),
                              events=[dict(at=60,action='snapshot',pid=env.host.pid,reset_hwm=True)])
                    save(output/'load-spec.json',spec);progress(pair=index,role=role,stage='warmup60_measure300')
                    sender=env.spawn(1,['python3',HERE.parent/'5d-2/fload.py',output/'load-spec.json'],'load.log')
                    if sender.wait()!=0:raise RuntimeError('timed sender failed')
                    row['points'].append(point(env.host.pid));progress(pair=index,role=role,stage='idle30')
                    time.sleep(30);row['after_idle']=point(env.host.pid);row['points'].append(row['after_idle'])
                    _,text,_=env.api('GET','/api/v2/audit/stats');row['post_count']=json.loads(text)['total_queries']
                    after=output/'upstream-after';after.mkdir();metrics_snapshot(args.upstream_root,after)
                    row['changes']=protected_changes(work/'native',env.root,manifest);row['protected_ok']=row['changes']['ok']
                    (output/'udp-sockets.txt').write_text(pathlib.Path(f'/proc/{env.host.pid}/net/udp').read_text())
                row['cleanup']=json.loads((output/'cleanup.json').read_text());row['existing_identity']=json.loads((output/'existing-service-guard.json').read_text())['unchanged']
                progress(pair=index,role=role,stage='offline_metrics')
                row['timed']=read_summary(output/'load',60,300)
                events=json.loads((output/'load/events.json').read_text());row['events']=events
                if len(events)!=1 or events[0].get('error') or 'result' not in events[0]:raise ValueError('peak snapshot failed')
                row['points'].append(events[0]['result'])
                row['raw_sha256']={str(p.relative_to(output)):sha(p) for p in output.rglob('*') if p.is_file() and not p.is_relative_to(output/'runtime')}
                pair['roles'][role]=row;save(pair_root/'report.json',pair)
                print(index,role,json.dumps(row['timed']),flush=True)
                coverage=row['timed']['sent']==150000 and row['timed']['protocol_replies']==150000
                collection_only=args.collect_incomplete_sends and incomplete_send_collection_allowed(row['timed'])
                if (not coverage and not collection_only) or not row['protected_ok'] or not row['existing_identity'] or not all(row['cleanup'].values()):raise ValueError('incomplete observation; remaining slots not run')
            pair['paired_dns']=paired_dns(pair_root/'base',pair_root/'candidate')
            save(pair_root/'report.json',pair);report['pairs'].append(pair);save(args.output/'report.json',report)
        report['summary']=summarize(report['pairs']);progress(stage='completed')
        if args.collect_incomplete_sends and report['summary']['status']=='incomplete':
            report['unqualified_descriptive_metrics']=descriptive_metrics(report['pairs'])
            report['description_warning']='Numerical description only. Missing sends remain failed coverage; not timing acceptance.'
    except Exception as error:
        report['error']=f'{type(error).__name__}: {error}';progress(stage='failed',error=report['error']);raise
    finally:save(args.output/'report.json',report)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--source',type=pathlib.Path,required=True);p.add_argument('--base',type=pathlib.Path,required=True);p.add_argument('--candidate',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);p.add_argument('--method-head',required=True);p.add_argument('--upstream-root',type=pathlib.Path,default=ROOT)
    p.add_argument('--collect-incomplete-sends',action='store_true',help='collect later roles after unsent-only shortfalls; qualification remains failed')
    a=p.parse_args();run(a)
