import json,pathlib,subprocess,sys
ROOT=pathlib.Path('/root/rtdns-lossless-20261027');TOOLS=ROOT/'profile-collection/tools';UP=pathlib.Path('/opt/rtdns-test-upstream')
sys.path.insert(0,str(TOOLS/'test-upstream'))
from fixture import save,sha
from service import verify_fixture,NS
from check import exchange,validate
import dns.message,dns.zone
out=ROOT/'direct-scheduling-v1';out.mkdir(mode=0o700)
folder,manifest=verify_fixture(UP);endpoint=manifest['endpoints'][0]
zone=dns.zone.from_file(str(folder/f"zone-{endpoint['group']}.db"),origin='.',relativize=False)
# Runs inside the existing fixture namespace; direct CoreDNS UDP is the only DNS path.
for i in range(20):
 q=dns.message.make_query('a.fixture.test.','A');validate(exchange(q,endpoint,'udp',UP),q,zone,'udp')
pid=json.loads((UP/'runtime.json').read_text())['pid']
spec=dict(corpus=str(ROOT/'observations-collection-v1/corpus.json'),mode='weighted',rate=500,duration=360,seed=501,deadline_s=5,observation_only=True,sampling_process=True,sampler_cpu=1,precise_timer=True,retain_wire=True,pid=pid,mock_pid=pid,root=str(out),api_port=49099,dns_port=endpoint['ports']['udp'],output=str(out/'load'),events=[])
save(out/'spec.json',spec)
save(out/'identity.json',dict(scope='direct fixture sender diagnostic; no rtdns in path; no acceptance',wrapper_sha256=sha(ROOT/'sender-direct-scheduling.py'),controller_sha256=sha(pathlib.Path(__file__)),corpus_sha256=sha(pathlib.Path(spec['corpus'])),fixture_sha256=sha(folder/'manifest.json'),fixture_pid=pid,endpoint_port=spec['dns_port'],source_fload_sha256=sha(TOOLS/'5d-2/fload.py')))
subprocess.run(['nice','-n','-5','python3',ROOT/'sender-direct-scheduling.py',out/'spec.json'],check=True)
