#!/usr/bin/env python3
"""Fixed loopback responder ABBA control for measurement overhead, not rtdns."""
import argparse
import hashlib
import json
import os
import pathlib
import signal
import socket
import subprocess
import sys
import time

HERE=pathlib.Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'5d-2'))
import fload
from dnswire import answer
from real_metrics import read_summary


def server(output):
    with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
        sock.bind(('127.0.0.1',0))
        (output/'server.json').write_text(json.dumps(dict(port=sock.getsockname()[1],pid=os.getpid())))
        while True:
            wire,peer=sock.recvfrom(65535)
            sock.sendto(answer(wire,'192.0.2.1','2001:db8::1'),peer)


def run(output):
    output.mkdir(mode=0o700,parents=True)
    source=output/'corpus.json';source.write_text(json.dumps([dict(name='calibration.example',type=1,weight=1)]))
    report=dict(tools_sha256={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in [pathlib.Path(__file__).resolve(),pathlib.Path(fload.__file__)]},scope='measurement calibration only; fixed Python responder, not rtdns',whole_project_accepted=False,runs=[])
    with (output/'server.log').open('w') as log:
        child=subprocess.Popen(['taskset','-c','0','python3',str(pathlib.Path(__file__).resolve()),'--server',str(output)],stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
        try:
            deadline=time.monotonic()+10
            while not (output/'server.json').exists():
                if child.poll() is not None or time.monotonic()>deadline:raise RuntimeError('calibration server unavailable')
                time.sleep(.05)
            port=json.loads((output/'server.json').read_text())['port']
            for i,separate in enumerate([False,True,True,False],1):
                case=output/f'r{i}'
                spec=dict(corpus=str(source),output=str(case),observation_only=True,sampling_process=separate,precise_timer=separate,
                          mode='weighted',rate=500,duration=32,deadline_s=.5,seed=502,
                          pid=child.pid,mock_pid=child.pid,root=str(output),api_port=1,dns_port=port,events=[])
                fload.measure(spec)
                metrics=read_summary(case,2,30)
                report['runs'].append(dict(order=i,separate_sampler=separate,metrics=metrics))
                (output/'report.json').write_text(json.dumps(report,indent=2)+'\n')
                print(i,separate,metrics['reply_latency_ms'],flush=True)
        finally:
            if child.poll() is None:
                os.killpg(child.pid,signal.SIGTERM);child.wait(timeout=10)
            report['server_exited']=child.poll() is not None
            report['raw_sha256']={str(p.relative_to(output)):hashlib.sha256(p.read_bytes()).hexdigest() for p in output.rglob('*') if p.is_file() and p.name!='report.json'}
            (output/'report.json').write_text(json.dumps(report,indent=2)+'\n')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--server',type=pathlib.Path);parser.add_argument('--output',type=pathlib.Path)
    args=parser.parse_args()
    if args.server:server(args.server)
    elif args.output:run(args.output.resolve())
    else:parser.error('output required')
