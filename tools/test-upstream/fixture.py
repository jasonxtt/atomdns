#!/usr/bin/env python3
"""Generate versioned, offline CoreDNS fixtures; source config is never changed."""
import argparse
import hashlib
import json
import pathlib
import re
import sys
from urllib.parse import urlsplit

IP = '198.51.100.2'
VERSION = '1.15.0'
PROTOCOLS = {'udp': 'udp', 'tcp': 'tcp', 'tls': 'tls', 'dot': 'tls', 'https': 'https', 'doh': 'https', 'quic': 'quic', 'doq': 'quic'}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2)+'\n')


def map_entries(groups):
    values=[]
    for group, entries in groups.items():
        enabled=[e for e in entries if e.get('enabled',True)]
        if not enabled:
            raise ValueError('group has no enabled supplier: '+group)
        for rank, entry in enumerate(enabled):
            protocol=entry.get('protocol') or urlsplit(entry['addr']).scheme or 'udp'
            if protocol not in PROTOCOLS:raise ValueError('unsupported protocol: '+protocol)
            base=20000+len(values)*10
            if base+3>=60000:raise ValueError('too many endpoints')
            values.append(dict(id=f'p{len(values):03d}',group=group,tag=entry.get('tag',''),rank=rank,
                               protocol=PROTOCOLS[protocol],ports=dict(udp=base,tcp=base,tls=base+1,https=base+2,quic=base+3),
                               delay_ms=[0,30,80][min(rank,2)],address=IP))
    return values


def normalize_names(names):
    result=[];seen=set()
    for name in names:
        name=name.rstrip('.').lower()
        if not name or len(name)>253 or not all(re.fullmatch(r'[a-z0-9_-]{1,63}',part) for part in name.split('.')):
            raise ValueError('invalid fixture name')
        if name not in seen:result.append(name);seen.add(name)
    if not result:raise ValueError('empty fixture corpus')
    return result


def generate(output, groups, names):
    names=normalize_names(names)
    endpoints=map_entries(groups)
    output.mkdir(mode=0o700,parents=True)
    group_names=list(groups)
    for group_index,group in enumerate(group_names):
        if not re.fullmatch(r'[a-zA-Z0-9_-]+',group):raise ValueError('invalid group')
        v4={'domestic':'192.0.2','cnfake':'203.0.113','nocnfake':'192.0.2'}.get(group,'203.0.113')
        v6={'domestic':'2001:db8:1','cnfake':'2001:db8:2','nocnfake':'2001:db8:3'}.get(group,'2001:db8:4')
        lines=['$ORIGIN .','$TTL 300','@ 60 IN SOA ns.fixture.test. hostmaster.fixture.test. 1 3600 600 86400 60','@ IN NS ns.fixture.test.','ns.fixture.test. IN A '+IP]
        for i,name in enumerate(names):
            n=int.from_bytes(hashlib.sha256(name.encode()).digest()[:4],'big')
            ttl=[60,300,600][n%3];count=1+(n%4);target=f'c{i}.data.fixture.test.' if n%5==0 else name+'.'
            if target!=name+'.':lines.append(f'{name}. {ttl} IN CNAME {target}')
            for j in range(count):
                value=20+(n+j)%200
                lines.extend([f'{target} {ttl} IN A {v4}.{value}',f'{target} {ttl} IN AAAA {v6}:{group_index+1:x}::{value:x}'])
        # Stable semantic probes, including large response and all three RR sections.
        lines.extend(['a.fixture.test. 300 IN A '+v4+'.10','a.fixture.test. 300 IN AAAA '+v6+'::10',
                      'alias.fixture.test. 300 IN CNAME a.fixture.test.',
                      'chain.fixture.test. 300 IN CNAME alias.fixture.test.',
                      'txt.fixture.test. 60 IN TXT "fixed payload" "second string"',
                      'mx.fixture.test. 300 IN MX 10 a.fixture.test.',
                      '_dns._tcp.fixture.test. 300 IN SRV 0 5 53 a.fixture.test.'])
        for i in range(32):lines.append(f'big.fixture.test. 300 IN TXT "{i:03d}'+('x'*237)+'"')
        (output/f'zone-{group}.db').write_text('\n'.join(lines)+'\n')
    blocks=[]
    for e in endpoints:
        for protocol in ['udp','tls','https','quic']:
            listen=f'.:{e["ports"][protocol]}' if protocol=='udp' else f'{protocol}://.:{e["ports"][protocol]}'
            tls='    tls ../../certs/cert.pem ../../certs/key.pem\n' if protocol!='udp' else ''
            blocks.append(f'''{listen} {{
    bind {IP}
{tls}    errors
    prometheus 0.0.0.0:19153
    template IN ANY servfail.fixture.test. {{
        rcode SERVFAIL
    }}
    file zone-{e['group']}.db . {{
        reload 0
    }}
}}
''')
    (output/'Corefile').write_text('\n'.join(blocks))
    save(output/'names.json',names)
    content={p.name:sha(p) for p in sorted(output.iterdir()) if p.is_file()}
    report=dict(schema=1,version=VERSION,names=len(names),qtypes=[1,28],endpoints=endpoints,content_sha256=content,
                records_sha256={k:v for k,v in content.items() if k.startswith('zone-')},offline=True)
    save(output/'manifest.json',report)
    return report


def from_source(source, output):
    sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]/'e2e-1'))
    from prepare import definitions
    sys.path.insert(0,str(pathlib.Path(__file__).resolve().parents[1]/'5d-1'))
    from load import corpus
    settings=json.loads((source/'webinfo/settings.json').read_text())
    groups={d['tag']:settings.get('upstreams',{}).get(d['tag'],d['args'].get('upstreams',[])) for d in definitions(source) if d['type']=='aliapi'}
    names=[v['name'] for v in corpus((source/'gen/top_domains.txt').read_text())]
    return generate(output,groups,names)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source',type=pathlib.Path);parser.add_argument('--groups',type=pathlib.Path);parser.add_argument('--names',type=pathlib.Path);parser.add_argument('--output',type=pathlib.Path,required=True)
    args=parser.parse_args()
    if args.source and (args.groups or args.names):parser.error('choose source or groups/names')
    if args.source:value=from_source(args.source.resolve(),args.output.resolve())
    elif args.groups and args.names:value=generate(args.output.resolve(),json.loads(args.groups.read_text()),json.loads(args.names.read_text()))
    else:parser.error('source or both groups/names required')
    print(json.dumps(dict(names=value['names'],endpoints=len(value['endpoints']),offline=value['offline'])))
