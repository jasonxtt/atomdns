"""Private prospective identical-BASE and fixed-seed source controls, not acceptance."""
import argparse,hashlib,json,os,pathlib,subprocess,sys,time
ROOT=pathlib.Path('/root/rtdns-lossless-20261027');TOOLS=ROOT/'profile-collection/tools';UP=pathlib.Path('/opt/rtdns-test-upstream')
sys.path.insert(0,str(TOOLS/'test-upstream'))
from case import Case
from fixture import sha,save
from performance import protected_changes
from real_metrics import read_summary

def main():
 out=ROOT/'cause-controls-v1'
 out.mkdir(mode=0o700)
 work=ROOT/'observations-collection-v1/work'
 manifest=json.loads((work/'input-manifest.json').read_text())
 corpus=ROOT/'observations-collection-v1/corpus.json';sequence=ROOT/'observations-collection-v1/fill-sequence.json'
 binaries={'base':ROOT/'binaries/base','candidate':ROOT/'binaries/candidate'}
 tools={str(p.relative_to(TOOLS)):sha(p) for p in TOOLS.rglob('*.py')}
 artifact={k:sha(v) for k,v in binaries.items()}
 assert artifact=={'base':'ada3564b5bf0430e3332784f0b807dced6619d62faaca920ab736c30d018917b','candidate':'03d6ccad5440e0996b1e7ff0e8cec13f2abe5340b059f8bfc262ce0ec98e9dc7'}
 plan=[('control-a','base'),('control-b','base'),('candidate-a','candidate'),('base-a','base'),('candidate-b','candidate'),('base-b','base')]
 report=dict(scope='diagnostic only, same seed501 in all roles; original acceptance unchanged',controller_sha=sha(pathlib.Path(__file__)),tools_sha=tools,binaries=artifact,input_sha=sha(work/'input-manifest.json'),corpus_sha=sha(corpus),schedule=dict(warmup=60,measure=300,rate=500,fill=100000),plan=plan,roles=[])
 save(out/'report.json',report)
 for label,role in plan:
  assert tools=={str(p.relative_to(TOOLS)):sha(p) for p in TOOLS.rglob('*.py')}
  assert artifact=={k:sha(v) for k,v in binaries.items()}
  save(out/'progress.json',dict(label=label,role=role,stage='start',wall=time.time()))
  result={};dest=out/label
  with Case(work,binaries[role],dest,UP) as env:
   fs=dict(sequence=str(sequence),count=100000,raw=str(dest/'fill-dns.jsonl.gz'),summary=str(dest/'fill-summary.json'))
   save(dest/'fill-spec.json',fs)
   sender=env.spawn(1,['python3',TOOLS/'memory-validation/real_config_probe.py','--fill-worker',dest/'fill-spec.json'],'fill.log')
   if sender.wait()!=0:raise RuntimeError('prefill failure')
   filled=json.loads((dest/'fill-summary.json').read_text());assert filled['planned']==filled['received']==100000
   status,text,_=env.api('GET','/api/v2/audit/stats');assert status==200 and json.loads(text)['total_queries']==100000
   spec=dict(corpus=str(corpus),mode='weighted',rate=500,duration=360,seed=501,deadline_s=5,observation_only=True,sampling_process=True,sampler_cpu=1,extra_pids={'proxy':env.mock.pid},precise_timer=True,retain_wire=True,pid=env.host.pid,mock_pid=json.loads((UP/'runtime.json').read_text())['pid'],root=str(env.root),api_port=49099,dns_port=40053,output=str(dest/'load'),events=[dict(at=60,action='snapshot',pid=env.host.pid,reset_hwm=True)])
   save(dest/'load-spec.json',spec)
   (dest/'system-stat-before.txt').write_text(pathlib.Path('/proc/stat').read_text())
   (dest/'cpuinfo-before.txt').write_text(pathlib.Path('/proc/cpuinfo').read_text())
   save(out/'progress.json',dict(label=label,role=role,stage='warmup60_measure300',wall=time.time()))
   sender=env.spawn(1,['python3',TOOLS/'5d-2/fload.py',dest/'load-spec.json'],'load.log')
   if sender.wait()!=0:raise RuntimeError('sender failed')
   (dest/'system-stat-after.txt').write_text(pathlib.Path('/proc/stat').read_text())
   (dest/'cpuinfo-after.txt').write_text(pathlib.Path('/proc/cpuinfo').read_text())
   (dest/'udp-sockets.txt').write_text(pathlib.Path(f'/proc/{env.host.pid}/net/udp').read_text())
   result['protected']=protected_changes(work/'native',env.root,manifest);assert result['protected']['ok']
   status,text,_=env.api('GET','/api/v2/audit/stats');assert status==200
   result['after_stats']=json.loads(text);assert result['after_stats']['total_queries']==100000
   save(dest/'result-in-run.json',result)
  result['timed']=read_summary(dest/'load',60,300)
  result['cleanup']=json.loads((dest/'cleanup.json').read_text());assert all(result['cleanup'].values())
  result['existing_identity']=json.loads((dest/'existing-service-guard.json').read_text())['unchanged'];assert result['existing_identity']
  result['label']=label;result['role']=role
  result['raw_sha']={str(p.relative_to(dest)):sha(p) for p in dest.rglob('*') if p.is_file() and not p.is_relative_to(dest/'runtime')}
  report['roles'].append(result);save(out/'report.json',report)
  print(label,json.dumps(result['timed']),flush=True)
 save(out/'progress.json',dict(stage='completed',wall=time.time()))
if __name__=='__main__':main()
