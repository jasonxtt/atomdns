"""One isolated rtdns test case connected only to the fixture bridge."""
import fcntl
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import time

from fixture import sha,save
from service import BRIDGE,OWNER,ROOT,links,verify_fixture
HERE=pathlib.Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'5d-2'))
from harness import Env,command,service_state
sys.path.insert(0,str(HERE.parent/'memory-validation'))
from run import existing_service_state


def cleanup_link(name):
    value=next((v for v in links() if v['ifname']==name),None)
    if value is None:return
    if value.get('ifalias')!='rtdns-test-case':raise RuntimeError('test interface ownership changed')
    try:command(['ip','link','del',name])
    except subprocess.CalledProcessError:
        # netns deletion destroys its veth asynchronously. Only the proven
        # already-absent device is benign; persistent/ownership errors propagate.
        if any(v['ifname']==name for v in links()):raise


class Case(Env):
    def __init__(self,work,binary,output,upstream_root=ROOT):
        super().__init__(work,'native',binary,output)
        self.upstream_root=upstream_root;self.ns=f'rtup-case-{os.getpid()}';self.host_link=f'rts{os.getpid()}';self.peer_link=f'rtp{os.getpid()}'
        self.api_port=49099;self.lock=None;self.network_ready=False;self.output_created=False;self.old_signals={}

    def __enter__(self):
        self.lock=(self.upstream_root/'case.lock').open('a')
        try:
            fcntl.flock(self.lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            self.output.mkdir(mode=0o700,parents=True);self.output_created=True
            self.old_signals={sig:signal.signal(sig,self._interrupted) for sig in [signal.SIGTERM,signal.SIGINT]}
            self.before=service_state();self.identity_before=existing_service_state()
            expected=json.loads((self.work/'input-manifest.json').read_text())['prepared']
            actual={str(p.relative_to(self.work/'native')):sha(p) for p in (self.work/'native').rglob('*') if p.is_file()}
            if actual!=expected:raise ValueError('prepared input fingerprint mismatch')
            shutil.copytree(self.work/'native',self.root)
            self.env=dict(os.environ,SSL_CERT_FILE=str(self.root/'ca.pem'),SSL_CERT_DIR=str(self.root/'empty-certs'),PYTHONDONTWRITEBYTECODE='1')
            for key in ['HTTP_PROXY','HTTPS_PROXY','ALL_PROXY','http_proxy','https_proxy','all_proxy']:
                self.env.pop(key,None)
            _,fixture=verify_fixture(self.upstream_root)
            if fixture!=json.loads((self.work/'fixture-manifest.json').read_text()):raise ValueError('active fixture identity changed')
            command(['python3',HERE/'service.py','reset','--root',self.upstream_root])
            bridge=next(v for v in links() if v['ifname']==BRIDGE)
            if bridge.get('ifalias')!=OWNER:raise ValueError('fixture bridge owner differs')
            if any(v['ifname'] in {self.host_link,self.peer_link} for v in links()):raise ValueError('test interface name occupied')
            command(['ip','netns','add',self.ns]);self.created=True
            command(['ip','link','add',self.host_link,'type','veth','peer','name',self.peer_link]);self.network_ready=True
            command(['ip','link','set',self.host_link,'alias','rtdns-test-case'])
            command(['ip','link','set',self.host_link,'master',BRIDGE]);command(['ip','link','set',self.host_link,'mtu','9000']);command(['ip','link','set',self.host_link,'up'])
            command(['ip','link','set',self.peer_link,'netns',self.ns])
            command(['ip','-n',self.ns,'link','set','lo','up']);command(['ip','-n',self.ns,'link','set',self.peer_link,'mtu','9000'])
            command(['ip','-n',self.ns,'addr','add','198.51.100.3/24','dev',self.peer_link]);command(['ip','-n',self.ns,'link','set',self.peer_link,'up'])
            self.mock=self.spawn(2,['python3',HERE/'proxy.py','--manifest',self.work/'fixture-manifest.json','--stats',self.output/'proxy-stats.json'],'proxy.log')
            deadline=time.monotonic()+10
            while not (self.output/'proxy-stats.json').exists():
                if self.mock.poll() is not None or time.monotonic()>deadline:raise RuntimeError('test proxy unavailable')
                time.sleep(.05)
            self.host=self.spawn(0,['unshare','--mount','--propagation','private','sh','-c','mount --bind "$1" /tmp; shift; exec "$@"','fixture-case',self.root/'tmp',self.binary.resolve(),'start','-d',self.root,'-c','config_custom.yaml'],'host.log')
            deadline=time.monotonic()+90
            while True:
                if self.host.poll() is not None:raise RuntimeError('rtdns exited before ready')
                try:
                    status,text,_=self.api('GET','/api/v2/audit/stats',timeout=1)
                    if status==200:break
                except subprocess.CalledProcessError:pass
                if time.monotonic()>deadline:raise RuntimeError('rtdns readiness timeout')
                time.sleep(.1)
            status,_,_=self.api('POST','/api/v1/audit/start',{})
            if status!=200:raise RuntimeError('audit start failed')
            routes=json.loads(subprocess.check_output(['ip','-n',self.ns,'-j','route'],text=True))
            if any(r['dst']=='default' for r in routes):raise ValueError('test case has default route')
            save(self.output/'identity.json',dict(binary_sha256=sha(self.binary),sut_pid=self.host.pid,proxy_pid=self.mock.pid,namespace=self.ns,routes=routes))
            return self
        except BaseException:
            self.__exit__(*sys.exc_info());raise

    def __exit__(self,*exc):
        try:
            # Existing harness kills/waits all owned process groups, checks namespace
            # emptiness, deletes it and protects the pre-existing mosdns service.
            if self.output_created and hasattr(self,'before'):super().__exit__(*exc)
            if self.network_ready:cleanup_link(self.host_link)
            if hasattr(self,'identity_before'):
                unchanged=existing_service_state()==self.identity_before
                save(self.output/'existing-service-guard.json',dict(unchanged=unchanged))
                if not unchanged:raise RuntimeError('existing product identity changed')
        finally:
            if self.lock:self.lock.close();self.lock=None
            for sig,prior in self.old_signals.items():signal.signal(sig,prior)
            self.old_signals={}
        return False
