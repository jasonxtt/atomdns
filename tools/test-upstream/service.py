#!/usr/bin/env python3
"""Owned CoreDNS namespace lifecycle. Never changes host DNS/default routes/qdisc."""
import argparse
import json
import os
import pathlib
import re
import signal
import socket
import struct
import subprocess
import time

from fixture import IP, VERSION, sha, save

ROOT=pathlib.Path('/opt/atomdns-test-upstream')
NS='atomdns-upstream'
BRIDGE='rtup-br'
HOST='rtup-host'
PEER='rtup-peer'
OWNER='atomdns-test-upstream-v1'
UNIT='atomdns-test-upstream.service'


def call(argv, **kwargs):
    return subprocess.run(list(map(str,argv)),check=True,**kwargs)


def capture(argv):
    return call(argv,stdout=subprocess.PIPE,text=True).stdout


def verify_fixture(root):
    ident=(root/'active').read_text().strip()
    if not re.fullmatch(r'[a-zA-Z0-9_-]+',ident):raise ValueError('invalid fixture id')
    folder=root/'fixtures'/ident
    value=json.loads((folder/'manifest.json').read_text())
    if value['version']!=VERSION or not value['offline']:raise ValueError('invalid fixture version/offline declaration')
    for name,expected in value['content_sha256'].items():
        if pathlib.Path(name).name!=name or sha(folder/name)!=expected:raise ValueError('fixture fingerprint mismatch')
    return folder,value


def topology(endpoints):
    p=['ip','netns','exec',NS]
    commands=[['ip','link','add',BRIDGE,'type','bridge'],
              ['ip','link','set',BRIDGE,'alias',OWNER],
              ['ip','addr','add','198.51.100.1/24','dev',BRIDGE],['ip','link','set',BRIDGE,'up'],
              ['ip','netns','add',NS],['ip','link','add',HOST,'type','veth','peer','name',PEER],
              ['ip','link','set',HOST,'alias',OWNER],['ip','link','set',HOST,'master',BRIDGE],
              ['ip','link','set',HOST,'mtu','9000'],['ip','link','set',HOST,'up'],
              ['ip','link','set',PEER,'netns',NS],
              p+['ip','link','set','lo','up'],p+['ip','link','set',PEER,'mtu','9000'],
              p+['ip','addr','add',IP+'/24','dev',PEER],p+['ip','link','set',PEER,'up'],
              p+['tc','qdisc','add','dev',PEER,'root','handle','1:','prio','bands','3','priomap']+['0']*16,
              p+['tc','qdisc','add','dev',PEER,'parent','1:2','handle','20:','netem','delay','30ms','limit','1000'],
              p+['tc','qdisc','add','dev',PEER,'parent','1:3','handle','30:','netem','delay','80ms','limit','1000']]
    for e in endpoints:
        if e['delay_ms'] not in [0,30,80]:raise ValueError('unsupported delay profile')
        if not e['delay_ms']:continue
        band='1:2' if e['delay_ms']==30 else '1:3'
        for port in sorted(set(e['ports'].values())):
            for protocol in ['udp','tcp']:
                commands.append(p+['tc','filter','add','dev',PEER,'protocol','ip','parent','1:','prio','1','flower','ip_proto',protocol,'src_port',str(port),'classid',band])
    return commands


def links():
    return json.loads(capture(['ip','-j','link']))


def cleanup(root):
    path=root/'network-owner.json'
    if not path.exists():return
    owner=json.loads(path.read_text())
    present={v['ifname']:v for v in links()}
    for name in [HOST,BRIDGE]:
        if name in present and present[name].get('ifalias')!=OWNER:raise RuntimeError('refuse to delete unowned interface')
    net=pathlib.Path('/run/netns')/NS
    if net.exists() and owner.get('namespace_inode')!=net.stat().st_ino:raise RuntimeError('refuse to delete unowned namespace')
    if BRIDGE in present:
        ports=[v['ifname'] for v in present.values() if v.get('master')==BRIDGE or v.get('master')==present[BRIDGE]['ifindex']]
        if any(v!=HOST for v in ports):raise RuntimeError('bridge still has other attached test clients')
    if HOST in present:call(['ip','link','del',HOST])
    if net.exists():call(['ip','netns','del',NS])
    if BRIDGE in present:call(['ip','link','del',BRIDGE])
    path.unlink()
    (root/'runtime.json').unlink(missing_ok=True)


def serve(root):
    folder,manifest=verify_fixture(root)
    certificate=json.loads((root/'certs/identity.json').read_text())
    if sha(root/'certs/ca.pem')!=certificate['ca_sha256'] or sha(root/'certs/cert.pem')!=certificate['certificate_sha256']:
        raise ValueError('test certificate fingerprint mismatch')
    call(['openssl','x509','-in',root/'certs/cert.pem','-checkend','3600','-noout'],stdout=subprocess.DEVNULL)
    call(['openssl','verify','-x509_strict','-purpose','sslserver','-CAfile',root/'certs/ca.pem',root/'certs/cert.pem'],stdout=subprocess.DEVNULL)
    installation=json.loads((root/'install.json').read_text())
    if sha(root/'bin/coredns')!=installation['binary_sha256']:raise ValueError('CoreDNS binary fingerprint mismatch')
    present={v['ifname'] for v in links()}
    if present.intersection({HOST,BRIDGE,PEER}) or (pathlib.Path('/run/netns')/NS).exists():raise RuntimeError('owned network names already occupied; do not take over')
    import ipaddress
    network=ipaddress.ip_network('198.51.100.0/24')
    for route in json.loads(capture(['ip','-j','route'])):
        if route['dst']!='default' and network.overlaps(ipaddress.ip_network(route['dst'],strict=False)):
            raise RuntimeError('test network overlaps an existing route')
    owner={};save(root/'network-owner.json',owner)
    child=None
    def interrupted(signum,frame):raise InterruptedError('service stopping')
    signal.signal(signal.SIGTERM,interrupted)
    try:
        for argv in topology(manifest['endpoints']):
            call(argv)
            if argv==['ip','netns','add',NS]:
                owner['namespace_inode']=(pathlib.Path('/run/netns')/NS).stat().st_ino;save(root/'network-owner.json',owner)
        child=subprocess.Popen(['ip','netns','exec',NS,'taskset','-c','2',str(root/'bin/coredns'),'-conf','Corefile'],cwd=folder,
                               env=dict(os.environ,GOMAXPROCS='1'),start_new_session=True)
        save(root/'runtime.json',dict(pid=child.pid,fixture=folder.name,fixture_manifest_sha256=sha(folder/'manifest.json'),namespace_inode=owner['namespace_inode']))
        if child.wait()!=0:raise RuntimeError('CoreDNS exited with failure')
    except InterruptedError:
        pass
    finally:
        if child and child.poll() is None:
            os.killpg(child.pid,signal.SIGTERM)
            try:child.wait(timeout=10)
            except subprocess.TimeoutExpired:os.killpg(child.pid,signal.SIGKILL);child.wait()
        cleanup(root)


def wait_ready(root):
    _,value=verify_fixture(root)
    port=value['endpoints'][0]['ports']['udp']
    question=bytes.fromhex('beef01000001000000000000')+b'\x01a\x07fixture\x04test\x00'+bytes.fromhex('00010001')
    deadline=time.monotonic()+30
    while time.monotonic()<deadline:
        try:
            # Bind only after the owned bridge address exists; create a fresh
            # socket on retry so a pre-namespace route/source is never retained.
            with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as sock:
                sock.settimeout(.5);sock.bind(('198.51.100.1',0));sock.connect((IP,port))
                sock.send(question);answer=sock.recv(65535)
                ident,flags,questions,count=struct.unpack_from('!4H',answer)
                if ident==0xbeef and flags&0x8000 and flags&15==0 and questions==1 and count>0:return
        except (OSError,struct.error):pass
        time.sleep(.1)
    raise RuntimeError('CoreDNS did not become ready')


def install_unit(root):
    unit=pathlib.Path('/etc/systemd/system')/UNIT
    text=f'''[Unit]
Description=Isolated fixed-response atomdns test upstream
After=network.target

[Service]
Type=simple
ExecStart=/usr/bin/python3 {root}/tools/test-upstream/service.py serve --root {root}
ExecStopPost=/usr/bin/python3 {root}/tools/test-upstream/service.py cleanup --root {root}
Restart=no
TimeoutStopSec=20
UMask=0077

[Install]
WantedBy=multi-user.target
'''
    if unit.exists() and unit.read_text()!=text:raise ValueError('refuse to overwrite different unit')
    unit.write_text(text);call(['systemctl','daemon-reload'])


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=['serve','cleanup','install-unit','start','stop','reset','status','check'])
    parser.add_argument('--root',type=pathlib.Path,default=ROOT)
    args=parser.parse_args();root=args.root.resolve()
    if args.action=='serve':serve(root)
    elif args.action=='cleanup':cleanup(root)
    elif args.action=='install-unit':install_unit(root)
    elif args.action in ['start','stop','reset']:
        call(['systemctl',{'reset':'restart'}.get(args.action,args.action),UNIT])
        if args.action!='stop':
            try:wait_ready(root)
            except BaseException:
                call(['systemctl','stop',UNIT]);raise
    elif args.action=='check':
        folder,value=verify_fixture(root);print(json.dumps(dict(fixture=folder.name,names=value['names'],endpoints=len(value['endpoints']),manifest_sha256=sha(folder/'manifest.json'))))
    else:
        print(capture(['systemctl','show',UNIT,'--property=ActiveState,SubState,MainPID']).strip())
        if (root/'runtime.json').exists():print((root/'runtime.json').read_text())

if __name__=='__main__':main()
