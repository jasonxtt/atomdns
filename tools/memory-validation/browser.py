#!/usr/bin/env python3
"""Serve the copied WebUI for a bounded manual/browser workflow proof.

Loopback only; does not alter the supplied prepared config. Owns the host,
namespace and proxy and removes all processes on exit or after ten minutes.
"""
import argparse
import json
import pathlib
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent/'5d-2'))
from harness import Env  # noqa: E402

p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--work',type=pathlib.Path,required=True)
p.add_argument('--binary',required=True)
p.add_argument('--output',type=pathlib.Path,required=True)
p.add_argument('--port',type=int,default=19099)
a=p.parse_args()
with Env(a.work,'native',a.binary,a.output) as env:
    log=(a.output/'proxy.log').open('w');env.logs.append(log)
    relay=subprocess.Popen(['socat',f'TCP-LISTEN:{a.port},bind=127.0.0.1,reuseaddr,fork',
                            f'EXEC:ip netns exec {env.ns} socat STDIO TCP\\:127.0.0.1\\:9099'],
                           stdout=log,stderr=subprocess.STDOUT,start_new_session=True)
    env.owned.append(relay)
    (a.output/'ready.json').write_text(json.dumps(dict(ns=env.ns,pid=env.host.pid,proxy_pid=relay.pid,port=a.port)))
    deadline=time.monotonic()+600
    while time.monotonic()<deadline and not (a.output/'stop').exists():
        if env.host.poll() is not None or relay.poll() is not None:raise RuntimeError('browser rig exited')
        time.sleep(.5)
    (a.output/'final-stats.json').write_text(env.api('GET','/api/v2/audit/stats')[1])
