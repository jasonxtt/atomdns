import gc,gzip,json,pathlib,selectors,sys,time,resource,hashlib
ROOT=pathlib.Path('/root/rtdns-lossless-20261027')
sys.path.insert(0,str(ROOT/'profile-collection/tools/5d-2'))
import fload
# Diagnostic-only direct fixture target: no rtdns process participates.
source=pathlib.Path(fload.__file__).read_text()
marker="sock.connect(('127.0.0.1', spec.get('dns_port', 53)))"
assert source.count(marker)==1
exec(compile(source.replace(marker,"sock.connect(('198.51.100.2', spec.get('dns_port', 53)))"),fload.__file__,'exec'),fload.__dict__)

def run(spec):
 events=[];maximum={};gc_start={};gc_events=[]
 original_dumps=json.dumps;original_open=gzip.open;original_decode=fload.decode;original_selector=selectors.DefaultSelector
 def timed(name,fn,*args,**kwargs):
  before_usage=resource.getrusage(resource.RUSAGE_THREAD)
  before_sched=list(map(int,pathlib.Path('/proc/thread-self/schedstat').read_text().split()))
  wall=time.monotonic();cpu=time.thread_time()
  try:return fn(*args,**kwargs)
  finally:
   cpu=time.thread_time()-cpu;elapsed=time.monotonic()-wall
   after_sched=list(map(int,pathlib.Path('/proc/thread-self/schedstat').read_text().split()))
   after_usage=resource.getrusage(resource.RUSAGE_THREAD)
   detail=dict(runqueue_wait_s=(after_sched[1]-before_sched[1])/1e9,voluntary_switches=after_usage.ru_nvcsw-before_usage.ru_nvcsw,involuntary_switches=after_usage.ru_nivcsw-before_usage.ru_nivcsw,major_faults=after_usage.ru_majflt-before_usage.ru_majflt,output_blocks=after_usage.ru_oublock-before_usage.ru_oublock)
   if elapsed>maximum.get(name,{}).get('wall_s',0):maximum[name]={'began':wall,'wall_s':elapsed,'cpu_s':cpu,**detail}
   if elapsed>(.010 if name=='select' else .005) and len(events)<10000:events.append({'operation':name,'began':wall,'wall_s':elapsed,'cpu_s':cpu,**detail})
 def callback(phase,info):
  gen=info['generation']
  if phase=='start':gc_start[gen]=(time.monotonic(),time.thread_time())
  else:
   wall,cpu=gc_start.pop(gen)
   gc_events.append(dict(began=wall,wall_s=time.monotonic()-wall,cpu_s=time.thread_time()-cpu,**info))
 class Writer:
  def __init__(self,stream):self.stream=stream
  def write(self,text):return timed('gzip_write',self.stream.write,text)
  def __getattr__(self,name):return getattr(self.stream,name)
 class Selector(original_selector):
  def select(self,*args,**kwargs):return timed('select',super().select,*args,**kwargs)
 def open_writer(filename,*args,**kwargs):
  stream=original_open(filename,*args,**kwargs)
  return Writer(stream) if pathlib.Path(filename).name=='responses.jsonl.gz' else stream
 json.dumps=lambda *a,**kw:timed('json_dumps',original_dumps,*a,**kw)
 gzip.open=open_writer;fload.decode=lambda *a,**kw:timed('decode',original_decode,*a,**kw)
 selectors.DefaultSelector=Selector;gc.callbacks.append(callback)
 try:fload.measure(spec)
 finally:
  gc.callbacks.remove(callback);json.dumps=original_dumps;gzip.open=original_open;fload.decode=original_decode;selectors.DefaultSelector=original_selector
  pathlib.Path(spec['output']).joinpath('sender-diagnostic.json').write_text(original_dumps(dict(scope='instrumented direct CoreDNS; rtdns absent; not timing acceptance',maximum=maximum,slow_spans=events,gc_spans=gc_events),indent=2))

if __name__=='__main__':run(json.loads(pathlib.Path(sys.argv[1]).read_text()))
