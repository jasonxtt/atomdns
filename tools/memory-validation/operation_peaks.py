#!/usr/bin/env python3
"""Offline hot-apply/audit peaks. Private inputs and replies never belong in Git."""
import argparse
import gzip
import hashlib
import json
import math
import os
import pathlib
import shutil
import signal
import statistics
import subprocess
import sys
import time

HERE=pathlib.Path(__file__).resolve().parent
sys.path[:0]=[str(HERE),str(HERE.parent/'test-upstream'),str(HERE.parent/'5d-2'),str(HERE.parent/'5d-1'),str(HERE.parent/'e2e-1')]
from fixture import from_source,save,sha
from install import certificates
from prepare_case import prepare
from harness import Env,command,service_state
sys.path.insert(0,str(HERE))
from run import existing_service_state,point
from load import corpus,semantic
from operation_summary import dns_outcomes,memory_gate,event_gate,parse_api_response,validate_controls

OPS=[40,50,60,70,80,90]
RANKS=['stats','stats/windows','rank/domain','rank/client','rank/domain_set','rank/effective','rank/slowest']
CASE_HASHES={3972:'485223d323c507b796fb6e5a6c7e29b26c50b3f6310921b925450df3f0406d89',3973:'485223d323c507b796fb6e5a6c7e29b26c50b3f6310921b925450df3f0406d89',5187:'0aa0611258756c03b4fffc2804d6e7c0fb9c6ba4894b882471f7f46e6b2c078a'}


def peer_worker(root,output):
    import dns.message,dns.zone
    from check import exchange,validate
    fixture=json.loads((root/'work/fixture-manifest.json').read_text())
    attempts=[]
    for endpoint in [e for e in fixture['endpoints'] if e['rank']==0]:
        zone=dns.zone.from_file(str(root/'upstream/fixtures/frozen'/('zone-'+endpoint['group']+'.db')),origin='.',relativize=False)
        until=time.monotonic()+30
        while True:
            began=time.monotonic();row=dict(endpoint=endpoint['id'],protocol=endpoint['protocol'])
            try:
                q=dns.message.make_query('a.fixture.test.','A')
                answer=exchange(q,endpoint,endpoint['protocol'],root/'upstream')
                validate(answer,q,zone,endpoint['protocol']);row['passed']=True
            except Exception as error:row.update(passed=False,error=type(error).__name__)
            row['seconds']=time.monotonic()-began;attempts.append(row);save(output,attempts)
            if row['passed']:break
            if time.monotonic()>=until:raise RuntimeError('peer readiness deadline')
            time.sleep(.1)


class OperationEnv(Env):
    def __init__(self,root,binary,output):
        super().__init__(root/'work','native',binary,output)
        self.task_root=root;self.ns=f'rtpk-{os.getpid()}';self.api_port=49099
    def __enter__(self):
        self.prior_signals={s:signal.signal(s,self._interrupted) for s in [signal.SIGTERM,signal.SIGINT]}
        self.output.mkdir(mode=0o700,parents=True)
        self.before=service_state();self.identities=existing_service_state()
        self.env=dict(os.environ,GOMAXPROCS='1',SSL_CERT_FILE=str(self.root/'ca.pem'),SSL_CERT_DIR=str(self.root/'empty-certs'),PYTHONDONTWRITEBYTECODE='1')
        for key in ['HTTP_PROXY','HTTPS_PROXY','ALL_PROXY','http_proxy','https_proxy','all_proxy']:self.env.pop(key,None)
        try:
            expected=json.loads((self.work/'input-manifest.json').read_text())['prepared']
            if expected!={str(p.relative_to(self.work/'native')):sha(p) for p in (self.work/'native').rglob('*') if p.is_file()}:raise ValueError('prepared input identity changed')
            shutil.copytree(self.work/'native',self.root)
            command(['ip','netns','add',self.ns]);self.created=True
            command(['ip','-n',self.ns,'link','set','lo','up'])
            command(['ip','-n',self.ns,'addr','add','198.51.100.2/32','dev','lo'])
            routes=json.loads(subprocess.check_output(['ip','-n',self.ns,'-j','route'],text=True))
            if any(r['dst']=='default' for r in routes):raise RuntimeError('default route in test namespace')
            core=self.spawn(2,['sh','-c','cd "$1"; exec "$2" -conf Corefile','peak-core',self.task_root/'upstream/fixtures/frozen',self.task_root/'upstream/bin/coredns'],'coredns.log')
            self.mock=core
            self.spawn(2,['python3',HERE.parent/'test-upstream/proxy.py','--manifest',self.work/'fixture-manifest.json','--stats',self.output/'proxy.json'],'proxy.log')
            self.spawn(2,['socat','TCP-LISTEN:17892,bind=127.0.0.1,fork','TCP:127.0.0.1:17891'],'second-proxy.log')
            ready=self.spawn(1,['python3',HERE/'operation_peaks.py','peer','--root',self.task_root,'--output',self.output/'peer-ready.json'],'peer-ready.log')
            if ready.wait()!=0 or core.poll() is not None:raise RuntimeError('fixture readiness failed')
            if not (self.output/'proxy.json').exists():raise RuntimeError('proxy not ready')
            self.host=self.spawn(0,['unshare','--mount','--propagation','private','sh','-c','mount --bind "$1" /tmp; shift; exec "$@"','peaks',self.root/'tmp',self.binary,'start','-d',self.root,'-c','config_custom.yaml'],'host.log')
            until=time.monotonic()+90
            while True:
                if self.host.poll() is not None:raise RuntimeError('SUT exited')
                try:
                    if self.api('GET','/api/v2/audit/stats',timeout=1)[0]==200:break
                except subprocess.CalledProcessError:pass
                if time.monotonic()>until:raise RuntimeError('SUT readiness deadline')
                time.sleep(.1)
            if self.api('POST','/api/v1/audit/start',{})[0]!=200:raise RuntimeError('capture start failed')
            save(self.output/'identity.json',dict(binary_sha256=sha(self.binary),sut_pid=self.host.pid,namespace=self.ns,routes=routes))
            return self
        except BaseException:self.__exit__(*sys.exc_info());raise
    def __exit__(self,*args):
        try:
            super().__exit__(*args)
            same=self.identities==existing_service_state()
            save(self.output/'existing-service-guard.json',dict(unchanged=same))
            if not same:raise RuntimeError('existing product identity changed')
        finally:
            for s,prior in self.prior_signals.items():signal.signal(s,prior)
        return False


def api(env,method,path,body=None):
    status,text,seconds=env.api(method,path,body,timeout=120)
    return parse_api_response(method,status,text)


def udp_counters(env):
    code='''import json,pathlib,sys
p=pathlib.Path('/proc/'+sys.argv[1]+'/net');rows=(p/'snmp').read_text().splitlines();value={}
for h,v in zip(rows,rows[1:]):
 if h.startswith('Udp:') and v.startswith('Udp:'):value=dict(zip(h.split()[1:],map(int,v.split()[1:])))
value['SUT_socket_drops']=sum(int(line.split()[-1]) for name in ['udp','udp6'] for line in (p/name).read_text().splitlines()[1:] if int(line.split()[1].rsplit(':',1)[1],16)==40053)
print(json.dumps(value))'''
    return json.loads(subprocess.check_output(['ip','netns','exec',env.ns,'python3','-c',code,str(env.host.pid)],text=True))


def make_events(env,name):
    original=dict(overrides=api(env,'GET','/api/v1/overrides'),flags=api(env,'GET','/api/v1/flags'),groups=api(env,'GET','/api/v1/special-groups'))
    requests=[]
    for index,at in enumerate(OPS if name!='M0' else []):
        e=dict(at=at,action='http',timeout=120)
        if name=='M1':
            body=dict(original['overrides']);body['socks5']='127.0.0.1:17892' if index%2==0 else original['overrides']['socks5']
            e.update(method='POST',path='/api/v1/overrides',body=body)
        elif name=='M3':
            if original['groups']:raise ValueError('M3 requires no existing dedicated groups')
            if index%2==0:e.update(method='POST',path='/api/v1/special-groups',body=dict(name='m3',listen_port=3999,custom_port_only=True,upstream_sources=[dict(kind='group',plugin_tag='foreign')]))
            else:e.update(method='DELETE',path='/api/v1/special-groups/50')
        elif name=='MF':
            flag=next(f for f in original['flags']['flags'] if f['key']=='prefer_v4')
            if flag['kind']!='bool' or flag['requery_on_change']:raise ValueError('MF requires bool prefer_v4 without requery')
            e.update(method='POST',path='/api/v1/flags/prefer_v4',body=dict(value=not flag['value'] if index%2==0 else flag['value']))
        requests.append(e)
    return requests+[dict(at=35,action='snapshot',pid=env.host.pid,reset_hwm=True),dict(at=145,action='snapshot',pid=env.host.pid)],original


def hot_run(root,scenario,role,pair,identity,progress,ignore_unsent=False):
    output=root/'results'/scenario/f'r{pair}'/role
    with OperationEnv(root,root/(role+'-rtdns'),output) as env:
        events,original=make_events(env,scenario)
        before=udp_counters(env)
        spec=dict(corpus=str(root/'work/corpus.json'),mode='weighted',rate=500,duration=150,seed=500+pair,deadline_s=5,observation_only=True,sampling_process=True,sampler_cpu=3,precise_timer=True,retain_wire=True,pid=env.host.pid,mock_pid=env.mock.pid,root=str(env.root),api_port=env.api_port,dns_port=40053,output=str(output/'load'),events=events)
        save(output/'load-spec.json',spec);progress(stage='load150')
        sender=env.spawn(1,['python3',HERE.parent/'5d-2/fload.py',output/'load-spec.json'],'sender.log')
        if sender.wait()!=0:raise RuntimeError('sender failed')
        after_load=point(env.host.pid);after=udp_counters(env)
        progress(stage='idle300');idle_began=time.monotonic();time.sleep(300);idle=point(env.host.pid)
        restored=dict(overrides=api(env,'GET','/api/v1/overrides')==original['overrides'],groups=api(env,'GET','/api/v1/special-groups')==original['groups'])
        old={f['key']:f['value'] for f in original['flags']['flags']};now={f['key']:f['value'] for f in api(env,'GET','/api/v1/flags')['flags']};restored['flags']=old==now
        meta=json.loads((output/'load/summary.json').read_text());done=json.loads((output/'load/events.json').read_text())
        with gzip.open(output/'load/requests.jsonl.gz','rt') as f:requests=[json.loads(line) for line in f]
        outcomes=dns_outcomes(requests,ignore_unsent=ignore_unsent)
        snapshots={e['at']:e['result'] for e in done if e['action']=='snapshot'}
        drops={k:after.get(k,0)-before.get(k,0) for k in ['InErrors','RcvbufErrors','SndbufErrors','SUT_socket_drops']}
        windows=[]
        for at in OPS:
            lat=sorted((r['finished']-r['planned'])*1000 for r in requests if meta['start']+at<=r['planned']<meta['start']+at+15 and r['finished'] is not None)
            windows.append(dict(at=at,until=at+15,p99_ms=lat[math.ceil(len(lat)*.99)-1] if lat else None))
        row=dict(outcomes=outcomes,drops=drops,orphan=meta['orphan_packets'],events_ok=event_gate(done,meta['start'],0 if scenario=='M0' else 6),restored=restored,events=done,rss145_kib=snapshots[145]['rss_kib'],peak_kib=max(snapshots[145]['hwm_kib'],after_load['hwm_kib']),after_load=after_load,idle=idle,idle_elapsed_s=time.monotonic()-idle_began,windows=windows,identity=identity)
    row['cleanup']=json.loads((output/'cleanup.json').read_text())
    row['ok']=len(requests)==75000 and outcomes['ok'] and not any(drops.values()) and not row['orphan'] and row['events_ok'] and all(restored.values()) and all(row['cleanup'].values()) and row['peak_kib']<=262144
    save(output/'report.json',row)
    return row


def fill_ring(env,output,index):
    spec=dict(sequence=str(env.task_root/'work/fill-sequence.json'),count=100000,raw=str(output/f'fill-{index}.jsonl.gz'),summary=str(output/f'fill-{index}-summary.json'))
    save(output/f'fill-{index}-spec.json',spec)
    worker=env.spawn(1,['python3',HERE/'real_config_probe.py','--fill-worker',output/f'fill-{index}-spec.json'],f'fill-{index}.log')
    if worker.wait()!=0:raise RuntimeError('prefill failed')
    counts=json.loads((output/f'fill-{index}-summary.json').read_text())
    if counts['received']!=100000 or counts['rcodes']!={'0':100000}:raise RuntimeError('prefill incomplete or incorrect protocol outcome')
    if api(env,'GET','/api/v2/audit/stats')['total_queries']!=100000:raise RuntimeError('ring is not full')


def audit_run(root,role,pair,identity,progress):
    output=root/'results/R3'/f'r{pair}'/role;row=dict(operations={},identity=identity)
    with OperationEnv(root,root/(role+'-rtdns'),output) as env:
        progress(stage='prefill100k');fill_ring(env,output,1)
        operations=[('read-'+p,'GET','/api/v2/audit/'+p,None,10) for p in RANKS]+[('clear','POST','/api/v1/audit/clear',{},1),('capacity400k','POST','/api/v1/audit/capacity',dict(capacity=400000),1),('capacity100k','POST','/api/v1/audit/capacity',dict(capacity=100000),1)]
        for name,method,path,body,count in operations:
            if name=='capacity400k':progress(stage='refill100k');fill_ring(env,output,2)
            progress(stage=name);points=[]
            for index in range(count):
                before=point(env.host.pid)
                pathlib.Path(f'/proc/{env.host.pid}/clear_refs').write_text('5')
                began=time.monotonic();value=api(env,method,path,body)
                if name=='clear' and api(env,'GET','/api/v2/audit/stats')['total_queries']!=0:raise RuntimeError('clear left retained queries')
                if name.startswith('capacity'):
                    if api(env,'GET','/api/v1/audit/capacity')['capacity']!=body['capacity']:raise RuntimeError('capacity readback mismatch')
                    if api(env,'GET','/api/v2/audit/stats')['total_queries']!=0:raise RuntimeError('resize did not clear')
                if path=='/api/v2/audit/stats' and value['total_queries']!=100000:raise RuntimeError('overview count mismatch')
                after=point(env.host.pid);points.append(dict(before=before,after=after,seconds=time.monotonic()-began))
            row['operations'][name]=dict(points=points,peak_kib=max(p['after']['hwm_kib'] for p in points),rss_after_kib=points[-1]['after']['rss_kib'],functional_ok=True)
    row['cleanup']=json.loads((output/'cleanup.json').read_text());row['ok']=all(row['cleanup'].values())
    save(output/'report.json',row);return row


def initialize(args):
    root=args.root;root.mkdir(mode=0o700)
    up=root/'upstream';(up/'bin').mkdir(parents=True)
    for source,target in [(args.base,root/'base-rtdns'),(args.candidate,root/'candidate-rtdns'),(args.core,up/'bin/coredns')]:shutil.copy2(source,target)
    certificates(up);fixture=from_source(args.source,up/'fixtures/frozen')
    work=root/'work';work.mkdir();save(work/'fixture-manifest.json',fixture)
    manifest=prepare(args.source,work/'native',fixture,up/'certs',True);save(work/'input-manifest.json',manifest)
    rows=[v for v in corpus((work/'native/gen/top_domains.txt').read_text()) if v['kind']=='production']
    for i,h in CASE_HASHES.items():
        if hashlib.sha256(rows[i]['name'].encode()).hexdigest()!=h:raise ValueError('known-exception corpus changed')
    save(work/'corpus.json',rows);save(work/'fill-sequence.json',rows[:128])


def paired_dns(folder):
    previous={};seen=set();differences=[];compared=0
    for role in ['base','candidate']:
        with gzip.open(folder/role/'load/responses.jsonl.gz','rt') as stream:
            for line in stream:
                row=json.loads(line);digest=hashlib.sha256(semantic(row['dns']).encode()).digest()
                if role=='base':previous[row['seq']]=digest
                else:
                    seen.add(row['seq'])
                    if row['seq'] in previous:
                        compared+=1
                        if previous[row['seq']]!=digest:differences.append(dict(seq=row['seq'],corpus_index=row['index']))
    return dict(compared=compared,missing_candidate=len(previous.keys()-seen),missing_base=len(seen-previous.keys()),differences=len(differences),first_differences=differences[:30],scope='DNS semantic equality only; not a correctness oracle; flag publication and learner expiry may change answers')


def run(root,scenarios=None,prior_report=None,ignore_unsent=False):
    if (root/'report.json').exists():raise ValueError('refuse to overwrite an existing batch')
    scenarios=['M0','M1','M3','MF','R3'] if scenarios is None else scenarios
    if not scenarios or len(set(scenarios))!=len(scenarios):raise ValueError('nonempty distinct scenarios required')
    tools=[p for folder in [HERE,HERE.parent/'test-upstream',HERE.parent/'5d-2',HERE.parent/'5d-1',HERE.parent/'e2e-1'] for p in folder.glob('*.py')]
    identity=dict(base=sha(root/'base-rtdns'),candidate=sha(root/'candidate-rtdns'),input=sha(root/'work/input-manifest.json'),corpus=sha(root/'work/corpus.json'),core=sha(root/'upstream/bin/coredns'),tools={str(p.relative_to(HERE.parent)):sha(p) for p in tools})
    controls=None;source_controls=None
    if prior_report is not None:
        prior=json.loads(prior_report.read_text());validate_controls(prior,identity)
        controls=prior['R2']['M0']
        source_controls=dict(report_sha256=sha(prior_report),M0={p:{r:dict(rss145_kib=roles[r]['rss145_kib'],idle_rss_kib=roles[r]['idle']['rss_kib'],ok=roles[r]['ok']) for r in ['base','candidate']} for p,roles in controls.items()})
    elif 'M0' not in scenarios and any(s!='R3' for s in scenarios):raise ValueError('hot-apply continuation requires prior M0 controls')
    report=dict(identity=identity,known_failure_indices=sorted(CASE_HASHES),scenarios=scenarios,ignore_unsent=ignore_unsent,source_controls=source_controls,R2={},R3={},gates={},complete=False)
    save(root/'report.json',report)
    def progress(**v):save(root/'progress.json',dict(wall=time.time(),**v))
    try:
        for scenario in scenarios:
            if scenario not in ['M0','M1','M3','MF','R3']:raise ValueError('unknown scenario')
            section=report['R3'] if scenario=='R3' else report['R2'].setdefault(scenario,{})
            for pair in [1,2,3]:
                for role in (['candidate','base'] if pair==2 else ['base','candidate']):
                    if identity['tools']!={str(p.relative_to(HERE.parent)):sha(p) for p in tools} or identity[role]!=sha(root/(role+'-rtdns')):raise ValueError('frozen method/binary changed')
                    def mark(**v):progress(scenario=scenario,pair=pair,role=role,**v)
                    row=(audit_run(root,role,pair,identity,mark) if scenario=='R3' else hot_run(root,scenario,role,pair,identity,mark,ignore_unsent=ignore_unsent))
                    section.setdefault(str(pair),{})[role]=row;save(root/'report.json',report)
                    if not row['ok']:raise RuntimeError('slot failed: '+scenario+' '+str(pair)+' '+role)
                    if scenario not in ['M0','R3']:
                        control=(controls if controls is not None else report['R2']['M0'])[str(pair)][role]
                        row['rss145_ratio']=row['rss145_kib']/control['rss145_kib'];row['idle_ratio']=row['idle']['rss_kib']/control['idle']['rss_kib']
                        if row['idle_ratio']>1.2:raise RuntimeError('idle RSS exceeds matching M0 x1.2')
                    print(scenario,pair,role,'complete',flush=True)
                if scenario!='R3':
                    section[str(pair)]['paired_dns']=paired_dns(root/'results'/scenario/f'r{pair}')
                    save(root/'report.json',report)
            if scenario=='R3':
                report['gates']['R3']={name:memory_gate([section[str(p)]['base']['operations'][name]['peak_kib'] for p in [1,2,3]],[section[str(p)]['candidate']['operations'][name]['peak_kib'] for p in [1,2,3]],cap_kib=None) for name in section['1']['base']['operations']}
                ok=all(g['ok'] for g in report['gates']['R3'].values())
            else:
                ratios=[[section[str(p)][r]['idle_ratio'] for r in ['base','candidate']] for p in [1,2,3]] if scenario!='M0' else None
                gate=memory_gate([section[str(p)]['base']['peak_kib'] for p in [1,2,3]],[section[str(p)]['candidate']['peak_kib'] for p in [1,2,3]],ratios)
                report['gates'][scenario]=gate;ok=gate['ok']
            save(root/'report.json',report)
            if not ok:raise RuntimeError('scenario memory gate failed: '+scenario)
        report['complete']=True;progress(stage='completed')
    except Exception as e:
        report['error']=f'{type(e).__name__}: {e}';progress(stage='failed',error=report['error']);raise
    finally:save(root/'report.json',report)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('action',choices=['init','run','peer']);p.add_argument('--root',type=pathlib.Path,required=True)
    for key in ['source','base','candidate','core','output']:p.add_argument('--'+key,type=pathlib.Path)
    p.add_argument('--scenarios',nargs='+',choices=['M0','M1','M3','MF','R3'])
    p.add_argument('--prior-report',type=pathlib.Path)
    p.add_argument('--ignore-unsent',action='store_true',help='explicit user-approved sender-only waiver; preserve all counts')
    a=p.parse_args()
    if a.action=='init':initialize(a)
    elif a.action=='peer':peer_worker(a.root,a.output)
    else:run(a.root,a.scenarios,a.prior_report,a.ignore_unsent)
