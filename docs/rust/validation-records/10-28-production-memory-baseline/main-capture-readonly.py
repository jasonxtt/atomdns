"""One finite read-only production snapshot; stdout must be kept private."""
import base64,hashlib,json,os,pathlib,platform,re,statistics,subprocess,threading,time,urllib.request,urllib.parse
ROOT=pathlib.Path('/cus/rtdns')
SELECT=['config_custom.yaml','sub_config','rule','srs','adguard','gen','webinfo']
service=lambda:dict(line.split('=',1) for line in subprocess.check_output(['systemctl','show','rtdns','-p','MainPID','-p','ActiveState','-p','NRestarts','-p','ExecMainStartTimestamp'],text=True).splitlines())
initial=service();pid=int(initial['MainPID']);assert initial['ActiveState']=='active' and pid>0
PROC=pathlib.Path(f'/proc/{pid}')
start_ticks=(PROC/'stat').read_text().rsplit(')',1)[1].split()[19]
sha=lambda b:hashlib.sha256(b).hexdigest()

def memory():
 values={}
 for line in (PROC/'status').read_text().splitlines():
  key,_,value=line.partition(':')
  if key in ['VmRSS','VmHWM','RssAnon','RssFile','RssShmem','VmSwap','VmSize','Threads']:values[key]=int(value.split()[0])
 return dict(unix=time.time(),monotonic=time.monotonic(),**values)

def rollup():
 return {l.split(':',1)[0]:int(l.split()[1]) for l in (PROC/'smaps_rollup').read_text().splitlines()[1:]}

def inventory():
 files={}
 for item in SELECT:
  p=ROOT/item
  for q in ([p] if p.is_file() else sorted(p.rglob('*'))):
   if q.is_symlink():raise RuntimeError('selected input contains symlink')
   if not q.is_file():continue
   data=q.read_bytes();st=q.stat()
   files[str(q.relative_to(ROOT))]=dict(bytes=len(data),sha256=sha(data),mtime_ns=st.st_mtime_ns)
 return files

result=dict(schema_version=1,scope='read-only production observation, no load/clear/resize/restart',started_unix=time.time(),pid=pid,start_ticks=start_ticks,service_before=initial,version=subprocess.check_output(['/usr/local/bin/rtdns','version'],text=True).strip(),binary_sha256=sha((PROC/'exe').read_bytes()),disk_binary_sha256=sha(pathlib.Path('/usr/local/bin/rtdns').read_bytes()),platform=dict(kernel=platform.release(),machine=platform.machine(),libc=platform.libc_ver(),page_bytes=os.sysconf('SC_PAGE_SIZE'),clock_ticks_per_second=os.sysconf('SC_CLK_TCK'),cpu_count=os.cpu_count(),cpu_affinity=sorted(os.sched_getaffinity(pid)),meminfo={k:int(v.split()[0]) for k,v in (l.split(':',1) for l in pathlib.Path('/proc/meminfo').read_text().splitlines()) if k in ['MemTotal','MemAvailable','SwapTotal','SwapFree']}),http={},samples=[])
assert result['binary_sha256']==result['disk_binary_sha256']=='b0107decf05fe5dca298fbc4e009efaf03bacad8b595f284cd6abb48657f17c8'
result['uptime_seconds_start']=float(pathlib.Path('/proc/uptime').read_text().split()[0])-int(start_ticks)/os.sysconf('SC_CLK_TCK')
result['inputs_before']=inventory()
settings=json.loads((ROOT/'webinfo/webui_port_settings.json').read_text());port=settings['port'];base=f'http://127.0.0.1:{port}'
result['audit_capacity_settings']=json.loads((ROOT/'webinfo/audit_settings.json').read_text())
stop=threading.Event();phase=['before_reads'];errors=[]
def sampling():
 try:
  while not stop.is_set():result['samples'].append(dict(phase=phase[0],**memory()));stop.wait(.25)
 except Exception as e:errors.append(type(e).__name__+': '+str(e))
thread=threading.Thread(target=sampling);thread.start()

def get(label,path):
 phase[0]=label;before=memory();began=time.time()
 request=urllib.request.Request(base+path,method='GET')
 with urllib.request.urlopen(request,timeout=30) as r:
  body=r.read();status=r.status;headers={k:v for k,v in r.headers.items() if k.lower() in ['content-type','content-length','x-audit-distinct-domains']}
 result['http'][label]=dict(path=path,began_unix=began,ended_unix=time.time(),status=status,headers=headers,bytes=len(body),sha256=sha(body),before=before,after=memory(),body_base64=base64.b64encode(body).decode())
 if status!=200:raise RuntimeError('GET failed')
 return json.loads(body) if 'json' in headers.get('Content-Type','').lower() else body.decode()

try:
 result['smaps_before']=rollup();time.sleep(10)
 get('health','/api/v1/system/health')
 get('audit_status','/api/v1/audit/status')
 get('audit_stats_before','/api/v2/audit/stats')
 get('cache_inventory','/api/v1/cache/inventory')
 get('metrics_before','/metrics')
 get('audit_windows','/api/v2/audit/stats/windows')
 get('domain_rank','/api/v2/audit/rank/domain?limit=1')
 get('log_sample','/api/v2/audit/logs?page=1&limit=500')
 config=(ROOT/'sub_config/route_learner.yaml').read_text()
 blocks=re.split(r'(?=^\s*- tag:)',config,flags=re.M)
 learner_tags=[]
 for block in blocks:
  if re.search(r'^\s*type:\s*route_learner\s*$',block,re.M):
   m=re.search(r'^\s*- tag:\s*([^\s#]+)',block,re.M)
   if m:learner_tags.append(m.group(1).strip('\"\''))
 result['learner_tags']=learner_tags
 for i,tag in enumerate(learner_tags):get('learner_'+str(i),'/plugins/'+urllib.parse.quote(tag,safe='')+'/status')
 get('audit_stats_after','/api/v2/audit/stats');get('metrics_after','/metrics')
 phase[0]='after_reads';time.sleep(15);result['smaps_after']=rollup()
finally:
 stop.set();thread.join();result['sampling_errors']=errors
result['inputs_after']=inventory();result['service_after']=service()
result['same_process']=start_ticks==(PROC/'stat').read_text().rsplit(')',1)[1].split()[19] and initial['MainPID']==result['service_after']['MainPID']
result['final_memory']=memory();result['ended_unix']=time.time()
assert result['same_process'] and not errors
print(json.dumps(result))
