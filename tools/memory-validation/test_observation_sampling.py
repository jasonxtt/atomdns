"""Real loopback integration: sampling must be outside the sender process."""
import gzip
import json
import os
import pathlib
import socket
import sys
import tempfile
import threading
import unittest

HERE=pathlib.Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'5d-2'))
import fload
from dnswire import answer


@unittest.skipUnless(sys.platform=='linux','requires proc sampling')
class ObservationSampling(unittest.TestCase):
    def test_separate_sampler_keeps_complete_requests_without_an_oracle(self):
        server=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);server.bind(('127.0.0.1',0));server.settimeout(.05)
        stop=threading.Event()
        def serve():
            while not stop.is_set():
                try:wire,peer=server.recvfrom(65535)
                except socket.timeout:continue
                server.sendto(answer(wire,'192.0.2.1','2001:db8::1'),peer)
        thread=threading.Thread(target=serve);thread.start()
        try:
            with tempfile.TemporaryDirectory() as directory:
                root=pathlib.Path(directory);(root/'corpus.json').write_text(json.dumps([dict(name='x.example',type=1,weight=1)]))
                cpu=max(os.sched_getaffinity(0))
                spec=dict(retain_wire=True,sampler_cpu=cpu,extra_pids={'proxy':os.getpid()},corpus=str(root/'corpus.json'),output=str(root/'out'),observation_only=True,sampling_process=True,precise_timer=True,
                          rate=20,duration=.5,deadline_s=.5,mode='weighted',pid=os.getpid(),mock_pid=os.getpid(),
                          api_port=1,dns_port=server.getsockname()[1],root=str(root),events=[])
                fload.measure(spec)
                meta=json.loads((root/'out/sampler.json').read_text())
                self.assertNotEqual(meta['sampler_pid'],os.getpid())
                self.assertEqual(meta['sender_pid'],os.getpid())
                self.assertEqual(meta['affinity'],[cpu])
                self.assertFalse(pathlib.Path(f"/proc/{meta['sampler_pid']}").exists())
                self.assertGreaterEqual(meta['sampler_nice'],10)
                with gzip.open(root/'out/requests.jsonl.gz','rt') as source:rows=[json.loads(line) for line in source]
                self.assertEqual(len(rows),10)
                self.assertTrue(all(r['status']=='observed_noerror' for r in rows))
                with gzip.open(root/'out/responses.jsonl.gz','rt') as source:responses=[json.loads(line) for line in source]
                self.assertEqual(len(responses),10)
                self.assertTrue(all(bytes.fromhex(r['wire_hex']) for r in responses))
                self.assertFalse(json.loads((root/'out/summary.json').read_text())['correctness_checked'])
                self.assertTrue(json.loads((root/'out/summary.json').read_text())['precise_timer'])
                samples=json.loads((root/'out/resources.json').read_text())
                self.assertGreater(len(samples),1)
                self.assertTrue(all('proxy' in s['processes'] for s in samples))
        finally:
            stop.set();thread.join();server.close()
