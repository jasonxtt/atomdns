#!/usr/bin/env python3
"""Bounded real-upstream production-copy observation, not formal acceptance."""
import argparse
import hashlib
import gzip
import sys
import collections
import json
import os
import pathlib
import re
import shutil
import signal
import socket
import subprocess
import time
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
API_PORT = 49099
OFFSET = 40000


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n')


def documents(root):
    import yaml
    main = root/'config_custom.yaml'
    result = [(main, yaml.safe_load(main.read_text()))]
    result += [(root/name, yaml.safe_load((root/name).read_text())) for name in result[0][1].get('include', [])]
    return result


def effective_upstreams(root):
    settings = json.loads((root/'webinfo/settings.json').read_text())
    return dict(proxies=settings.get('proxies', {}), upstreams={d['tag']:settings.get('upstreams',{}).get(d['tag'],d.get('args',{}).get('upstreams',[]))
        for _, doc in documents(root) for d in doc.get('plugins',[]) if d['type']=='aliapi'})


def prepare(source, output):
    import yaml
    if output.exists():
        raise ValueError('output must be new')
    before = effective_upstreams(source)
    shutil.copytree(source, output)
    docs = documents(output)
    ports = {}
    for _, doc in docs:
        for d in doc.get('plugins',[]):
            if d['type'].endswith('_server'):
                listen = d['args']['listen']
                port = int(listen.rsplit(':',1)[1])
                if not 0 < port+OFFSET <= 65535:
                    raise ValueError('listener outside reserved remapping range')
                ports[port] = port+OFFSET
                d['args']['listen'] = f'127.0.0.1:{port+OFFSET}'
    def address(value):
        if not isinstance(value,str):
            return value
        def replace(m):
            old = int(m.group(2))
            new = API_PORT if old in {80,9099} and value.startswith('http') else ports.get(old,old)
            return m.group(1)+str(new)
        return re.sub(r'((?:127\.0\.0\.1|localhost):)([0-9]+)',replace,value)
    for path, doc in docs:
        if path.name=='config_custom.yaml':
            doc.setdefault('api',{})['http']=f'127.0.0.1:{API_PORT}'
        for d in doc.get('plugins',[]):
            if d['type']=='rewrite' and 'dns' in d.get('args',{}):
                d['args']['dns']=address(d['args']['dns'])
        path.write_text(yaml.safe_dump(doc,sort_keys=False,allow_unicode=True))
    special=output/'webinfo/special_upstream_groups.json'
    if special.exists() and json.loads(special.read_text()):
        raise ValueError('dedicated listener ports require explicit mapping before real probe')
    save(output/'webinfo/webui_port_settings.json',dict(port=API_PORT))
    requery=output/'webinfo/requeryconfig.json'
    if requery.exists():
        def walk(value):
            if isinstance(value,dict):return {k:walk(v) for k,v in value.items()}
            if isinstance(value,list):return [walk(v) for v in value]
            return address(value)
        save(requery,walk(json.loads(requery.read_text())))
    if effective_upstreams(output)!=before:
        raise ValueError('effective upstream or proxy changed')
    manifest=dict(mode='real production upstreams, no mock',listener_ports=ports,api_port=API_PORT,
                  effective_upstream_sha256=hashlib.sha256(json.dumps(before,sort_keys=True).encode()).hexdigest(),
                  source={str(p.relative_to(source)):sha(p) for p in source.rglob('*') if p.is_file()},
                  prepared={str(p.relative_to(output)):sha(p) for p in output.rglob('*') if p.is_file()})
    return manifest


def probe_port(port, kind):
    with socket.socket(socket.AF_INET, kind) as probe:
        if kind == socket.SOCK_STREAM:
            probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        probe.bind(('127.0.0.1', port))


def fill(spec):
    sys.path.insert(0,str(HERE.parent/'5d-1'))
    from load import question, decode
    sequence=json.loads(pathlib.Path(spec['sequence']).read_text())
    counts=collections.Counter()
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock, gzip.open(spec['raw'],'wt') as raw:
        sock.connect(('127.0.0.1',40053));sock.settimeout(5)
        for i in range(spec['count']):
            row=sequence[i % len(sequence)]
            began=time.time();sock.send(question(row['name'],row['type'],i % 65536));dns=decode(sock.recv(65535))
            if dns['id']!=i % 65536 or dns['questions']!=[(row['name']+'.',row['type'],1)]:raise ValueError('fill question/id mismatch')
            counts[str(dns['rcode'])]+=1
            raw.write(json.dumps(dict(index=i,started=began,finished=time.time(),dns=dns))+'\n')
    save(pathlib.Path(spec['summary']),dict(planned=spec['count'],received=sum(counts.values()),rcodes=dict(counts)))


def run(args):
    from state_replay import load_runner
    runner=load_runner()
    from load import corpus
    args.output.mkdir(mode=0o700,parents=True)
    prepared=args.output/'prepared'
    manifest=prepare(args.source,prepared)
    save(args.output/'input-manifest.json',manifest)
    full_values=[dict(v,corpus_index=i) for i,v in enumerate(corpus((prepared/'gen/top_domains.txt').read_text())) if v['kind']=='production']
    save(args.output/'weighted-corpus.json',full_values)
    values=full_values[:args.limit]
    timed=json.loads(args.timed_spec.read_text()) if args.timed_spec else None
    if timed and (timed['warmup']<0 or timed['duration']<=0 or timed['rate']<=0 or timed.get('idle',30)<0):
        raise ValueError('invalid timed observation schedule')
    sequence=values+values
    save(args.output/'sequence.json',sequence)
    tags=[d['tag'] for _,doc in documents(prepared) for d in doc.get('plugins',[]) if d['type']=='route_learner']
    before=runner.existing_service_state()
    report=dict(scope='bounded real upstream observation only',whole_project_accepted=False,timing_eligible=False,
                effective_upstream_sha256=manifest['effective_upstream_sha256'],planned_per_role=len(sequence),
                real_observation_ttl_ceiling=2**32-1, mock_default_ttl_ceiling_unchanged=300,
                sequence_sha256=sha(args.output/'sequence.json'),binaries={k:sha(v) for k,v in [('base',args.base),('candidate',args.candidate)]},
                timed_spec=timed,weighted_corpus_sha256=sha(args.output/'weighted-corpus.json'),weighted_questions=len(full_values),
                tools_sha256={p.name:sha(p) for p in [HERE/'real_config_probe.py',HERE/'real_metrics.py',HERE/'state_replay.py',HERE.parent/'5d-2/fload.py']},
                roles={},relay='test loopback5533 -> production LAN localdns5533; both UDP/TCP; source client is test host')
    def interrupted(signum, frame):
        raise InterruptedError('real observation interrupted')
    signal.signal(signal.SIGTERM,interrupted)
    owned=[];streams=[]
    def spawn(argv,log):
        stream=log.open('w');streams.append(stream)
        proc=subprocess.Popen(list(map(str,argv)),stdout=stream,stderr=subprocess.STDOUT,start_new_session=True)
        owned.append(proc)
        return proc
    def stop(proc):
        try:os.killpg(proc.pid,signal.SIGTERM)
        except ProcessLookupError:return
        try:proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid,signal.SIGKILL);proc.wait()
    try:
        # Never take over any occupied host socket, including a pre-existing local DNS.
        ports=set(map(int,manifest['listener_ports'].values()))|{API_PORT,5533,1053}
        for port in ports:
            for kind in [socket.SOCK_DGRAM,socket.SOCK_STREAM]:
                probe_port(port,kind)
        for proto in ['UDP4','TCP4']:
            spawn(['taskset','-c','1','socat','-T10',f'{proto}-LISTEN:5533,bind=127.0.0.1,fork',f'{proto}:{args.production_host}:5533'],args.output/f'relay-{proto}.log')
        time.sleep(.2)
        if any(p.poll() is not None for p in owned):raise RuntimeError('localdns relay failed')
        roles=[('base',args.base),('candidate',args.candidate)]
        if args.reverse:roles.reverse()
        report['role_order']=[r for r,_ in roles]
        for role,binary in roles:
            save(args.output/'progress.json',dict(role=role,stage='startup',time=time.time()))
            out=args.output/role;out.mkdir(mode=0o700);runtime=out/'runtime';shutil.copytree(prepared,runtime)
            (runtime/'cache').mkdir(exist_ok=True);(runtime/'tmp').mkdir()
            for path,h in manifest['prepared'].items():
                if sha(runtime/path)!=h:raise ValueError('runtime input identity changed')
            host=spawn(['taskset','-c','0','unshare','--mount','--propagation','private','sh','-c',
                        'mount --bind "$1" /tmp; shift; exec "$@"','real-config',runtime/'tmp',binary.resolve(),'start','-d',runtime,'-c','config_custom.yaml'],out/'host.log')
            metrics={}
            try:
                deadline=time.monotonic()+90
                while True:
                    if host.poll() is not None:raise RuntimeError('host exited during startup')
                    try:
                        with urllib.request.urlopen(f'http://127.0.0.1:{API_PORT}/api/v2/audit/stats',timeout=1) as response:
                            if response.status==200:break
                    except OSError:
                        if time.monotonic()>deadline:raise RuntimeError('host startup timeout')
                        time.sleep(.1)
                req=urllib.request.Request(f'http://127.0.0.1:{API_PORT}/api/v1/audit/start',data=b'{}',headers={'Content-Type':'application/json'})
                with urllib.request.urlopen(req,timeout=5) as response:response.read()
                time.sleep(args.settle)
                metrics['ready']=runner.point(host.pid)
                save(out/'worker-spec.json',dict(api_port=API_PORT,dns_port=40053,ttl_ceiling=2**32-1,learners=tags,sequence=str(args.output/'sequence.json'),output=str(out)))
                child=spawn(['taskset','-c','1','python3',HERE/'state_replay.py','--worker',out/'worker-spec.json'],out/'worker.log')
                metrics['worker_exit']=child.wait()
                metrics['after']=runner.point(host.pid)
                if args.fill_count and metrics['worker_exit']==0:
                    save(args.output/'progress.json',dict(role=role,stage='prefill100k',time=time.time()))
                    save(out/'fill-spec.json',dict(sequence=str(args.output/'sequence.json'),count=args.fill_count,
                         raw=str(out/'fill-dns.jsonl.gz'),summary=str(out/'fill-summary.json')))
                    sender=spawn(['taskset','-c','1','python3',HERE/'real_config_probe.py','--fill-worker',out/'fill-spec.json'],out/'fill.log')
                    metrics['fill_exit']=sender.wait()
                    metrics['filled_before_idle']=runner.point(host.pid)
                    time.sleep(args.settle)
                    metrics['filled_after_idle']=runner.point(host.pid)
                    summary=out/'fill-summary.json'
                    if summary.exists():metrics['fill_summary']=json.loads(summary.read_text())
                if timed:
                    from real_metrics import read_summary
                    if metrics['worker_exit']!=0 or args.fill_count and metrics.get('fill_exit')!=0:
                        raise RuntimeError('incomplete prefill before timed observation')
                    with urllib.request.urlopen(f'http://127.0.0.1:{API_PORT}/api/v2/audit/stats',timeout=5) as response:
                        metrics['pre_load_stats']=json.load(response)
                    if args.fill_count and metrics['pre_load_stats']['total_queries']!=100000:
                        raise RuntimeError('audit ring not full before timed observation')
                    spec=dict(corpus=str(args.output/'weighted-corpus.json'),mode='weighted',seed=timed['seed'],
                              observation_only=True,sampling_process=timed.get('sampling_process',False),precise_timer=timed.get('precise_timer',False),rate=timed['rate'],duration=timed['warmup']+timed['duration'],deadline_s=5,
                              pid=host.pid,mock_pid=owned[0].pid,root=str(runtime),api_port=API_PORT,dns_port=40053,
                              output=str(out/'load'),events=[dict(at=timed['warmup'],action='snapshot',pid=host.pid,reset_hwm=True)])
                    save(out/'load-spec.json',spec)
                    metrics['before_load']=runner.point(host.pid)
                    save(args.output/'progress.json',dict(role=role,stage='timed',time=time.time(),seconds=spec['duration']))
                    sender=spawn(['taskset','-c','1','python3',HERE.parent/'5d-2/fload.py',out/'load-spec.json'],out/'load.log')
                    metrics['load_exit']=sender.wait()
                    if metrics['load_exit']!=0:raise RuntimeError('timed observation sender failed')
                    metrics['timed']=read_summary(out/'load',timed['warmup'],timed['duration'])
                    metrics['load_events']=json.loads((out/'load/events.json').read_text())
                    metrics['after_load']=runner.point(host.pid)
                    save(args.output/'progress.json',dict(role=role,stage='idle',time=time.time(),seconds=timed.get('idle',30)))
                    time.sleep(timed.get('idle',30))
                    metrics['after_idle']=runner.point(host.pid)
                with urllib.request.urlopen(f'http://127.0.0.1:{API_PORT}/api/v2/audit/stats',timeout=5) as response:metrics['audit_stats']=json.load(response)
                trace=out/'trace.jsonl'
                metrics['recorded']=len(trace.read_text().splitlines()) if trace.exists() else 0
                metrics['trace_sha256']=sha(trace) if trace.exists() else None
            finally:
                stop(host)
                report['roles'][role]=metrics
                save(args.output/'report.json',report)
    finally:
        for proc in reversed(owned):stop(proc)
        for stream in streams:stream.close()
        report['cleanup']=dict(all_owned_parents_exited=all(p.poll() is not None for p in owned),
                               existing_service_identity_unchanged=runner.existing_service_state()==before)
        # Catch surviving socat fork children in the owned relay process groups.
        groups={p.pid for p in owned}
        survivors=[]
        for p in pathlib.Path('/proc').iterdir():
            if p.name.isdigit():
                try:
                    fields=(p/'stat').read_text().rsplit(')',1)[1].split()
                    if int(fields[2]) in groups and fields[0]!='Z':survivors.append(int(p.name))
                except (FileNotFoundError,ProcessLookupError):pass
        report['cleanup']['owned_live_group_members']=survivors
        save(args.output/'report.json',report)


if __name__=='__main__':
    if len(sys.argv)==3 and sys.argv[1]=='--fill-worker':
        fill(json.loads(pathlib.Path(sys.argv[2]).read_text()));sys.exit(0)
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True)
    p.add_argument('--base',type=pathlib.Path,required=True);p.add_argument('--candidate',type=pathlib.Path,required=True)
    p.add_argument('--timed-spec',type=pathlib.Path);p.add_argument('--reverse',action='store_true')
    p.add_argument('--fill-count',type=int,choices=[0,100000],default=0)
    p.add_argument('--production-host',required=True);p.add_argument('--limit',type=int,default=128);p.add_argument('--settle',type=float,default=5)
    a=p.parse_args()
    if not 0<a.limit<=1000 or a.settle<0:p.error('limit1..1000; settle nonnegative')
    for key in ['source','output','base','candidate']:setattr(a,key,getattr(a,key).resolve())
    run(a)
