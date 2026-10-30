#!/usr/bin/env python3
"""Actual protocol, fixed-content, truncation and optional exhaustive UDP checks."""
import argparse
import asyncio
import gzip
import hashlib
import json
import pathlib
import ssl
import sys
import subprocess
import time

from fixture import IP, save, sha
from service import ROOT, NS, PEER, verify_fixture


def projection(response):
    return dict(rcode=response.rcode(),question=[x.to_text() for x in response.question],
                answer=[r.to_text() for r in response.answer],authority=[r.to_text() for r in response.authority],
                additional=[r.to_text() for r in response.additional],flags=response.flags)


def expected_answer(zone, name, rdtype):
    import dns.name,dns.rdatatype
    at=dns.name.from_text(name);result=[]
    for _ in range(8):
        cname=zone.get_rdataset(at,dns.rdatatype.CNAME)
        if cname is not None and rdtype!=dns.rdatatype.CNAME:
            result.append((at.to_text(),cname.ttl,int(dns.rdatatype.CNAME),[r.to_text() for r in cname]))
            at=next(iter(cname)).target
            continue
        data=zone.get_rdataset(at,rdtype)
        if data:result.append((at.to_text(),data.ttl,int(rdtype),[r.to_text() for r in data]))
        return result
    raise ValueError('fixture CNAME cycle')


def validate(response, query, zone, mode):
    import dns.flags,dns.rcode
    if response.question!=query.question or response.id!=(0 if mode=='quic' else query.id):raise ValueError('question/ID mismatch')
    if not response.flags & dns.flags.QR:raise ValueError('not a DNS response')
    name=query.question[0].name.to_text();kind=query.question[0].rdtype
    if name=='servfail.fixture.test.':
        if response.rcode()!=2 or response.answer:raise ValueError('wrong SERVFAIL fixture')
        return
    wanted=expected_answer(zone,name,kind)
    found=[(r.name.to_text(),r.ttl,int(r.rdtype),[x.to_text() for x in r]) for r in response.answer]
    if found!=wanted:raise ValueError('fixed answer/TTL/order mismatch')
    exists=zone.get_node(query.question[0].name) is not None
    if response.rcode()!=(0 if exists else 3):raise ValueError('wrong negative response code')
    if not wanted and not any(r.rdtype==6 for r in response.authority):raise ValueError('negative response missing SOA')
    # Every returned authority/additional RR must belong to the frozen zone.
    for rr in response.authority+response.additional:
        data=zone.get_rdataset(rr.name,rr.rdtype)
        if data is None or [r.to_text() for r in rr]!=[r.to_text() for r in data]:raise ValueError('unexpected authority/additional data')
        ttl=min(data.ttl,next(iter(data)).minimum) if rr.rdtype==6 and not wanted else data.ttl
        if rr.ttl!=ttl:raise ValueError('unexpected authority/additional TTL')


def exchange(query, endpoint, mode, root):
    import dns.query
    port=endpoint['ports'][mode]
    if mode=='udp':return dns.query.udp(query,IP,port=port,timeout=3)
    if mode=='tcp':return dns.query.tcp(query,IP,port=port,timeout=3)
    if mode=='tls':return dns.query.tls(query,IP,port=port,timeout=3,server_hostname='localhost',ssl_context=ssl.create_default_context(cafile=str(root/'certs/ca.pem')))
    if mode=='https':return dns.query.https(query,f'https://localhost:{port}/dns-query',timeout=3,bootstrap_address=IP,verify=str(root/'certs/ca.pem'),http_version=dns.query.HTTPVersion.H2)
    return dns.query.quic(query,IP,port=port,timeout=3,verify=str(root/'certs/ca.pem'),server_hostname='localhost')


async def exhaustive(endpoint, zone, names, output):
    import dns.asyncquery,dns.message
    semaphore=asyncio.Semaphore(32);count=0;checksum=hashlib.sha256()
    with gzip.open(output,'wt') as raw:
        async def one(name,kind):
            nonlocal count
            async with semaphore:
                q=dns.message.make_query(name,kind);r=await dns.asyncquery.udp(q,IP,port=endpoint['ports']['udp'],timeout=3)
                validate(r,q,zone,'udp');view=projection(r);encoded=json.dumps(view,sort_keys=True)
                raw.write(json.dumps(dict(name=name,type=kind,dns=view))+'\n');checksum.update(encoded.encode());count+=1
        # Bounded tasks and fixed question order; completion order is deliberately not a sequence oracle.
        for offset in range(0,len(names),128):
            await asyncio.gather(*(one(name,kind) for name in names[offset:offset+128] for kind in [1,28]))
    return dict(queries=count,all_content_validated=True,completion_digest=checksum.hexdigest(),raw_sha256=sha(output))


def check(root,output,full=False):
    import dns.message,dns.zone,dns.flags
    folder,manifest=verify_fixture(root);output.mkdir(mode=0o700,parents=True)
    began=time.time();report=dict(scope='CoreDNS infrastructure self-check; not atomdns performance acceptance',fixture_sha256=sha(folder/'manifest.json'),
                                  all_passed=False,protocols=[],exhaustive=[],whole_project_accepted=False,started=began,
                                  sources_sha256={p.name:sha(p) for p in [pathlib.Path(__file__).resolve(),pathlib.Path(__file__).with_name('fixture.py'),pathlib.Path(__file__).with_name('service.py')]},
                                  client=dict(python=sys.version,dnspython=__import__('dns').__version__,openssl=ssl.OPENSSL_VERSION),
                                  certificate=json.loads((root/'certs/identity.json').read_text()),
                                  core_dns=json.loads((root/'install.json').read_text())['binary_sha256'],
                                  limitation='DoQ large responses require an explicit EDNS buffer; no-EDNS large DoQ is not qualified')
    try:
        zones={e['group']:dns.zone.from_file(str(folder/f"zone-{e['group']}.db"),origin='.',relativize=False) for e in manifest['endpoints']}
        probes=[('a.fixture.test.','A'),('a.fixture.test.','AAAA'),('chain.fixture.test.','A'),('txt.fixture.test.','TXT'),
                ('txt.fixture.test.','AAAA'),('mx.fixture.test.','MX'),('_dns._tcp.fixture.test.','SRV'),
                ('absent.fixture.test.','A'),('servfail.fixture.test.','A')]
        for endpoint in manifest['endpoints']:
            for mode in ['udp','tcp','tls','https','quic']:
                for name,kind in probes:
                    prior=None
                    for repeat in range(2):
                        q=dns.message.make_query(name,kind);r=exchange(q,endpoint,mode,root);validate(r,q,zones[endpoint['group']],mode)
                        view=projection(r)
                        if prior is not None and prior!=view:raise ValueError('response ordering/content varies across identical queries')
                        prior=view
                q=dns.message.make_query('big.fixture.test.','TXT',use_edns=(mode=='quic'),payload=65535);r=exchange(q,endpoint,mode,root)
                if mode=='udp':
                    if not r.flags & dns.flags.TC:raise ValueError('large UDP answer not truncated')
                    q=dns.message.make_query('big.fixture.test.','TXT');r=exchange(q,endpoint,'tcp',root)
                validate(r,q,zones[endpoint['group']],'tcp' if mode=='udp' else mode)
                report['protocols'].append(dict(endpoint=endpoint['id'],protocol=mode,logical_cases=19,network_queries=20 if mode=='udp' else 19,passed=True,large_answer_edns=(mode=='quic')))
                save(output/'report.json',report)
            print(endpoint['id'],'protocols passed',flush=True)
        if full:
            names=json.loads((folder/'names.json').read_text())
            for endpoint in manifest['endpoints']:
                row=asyncio.run(exhaustive(endpoint,zones[endpoint['group']],names,output/(endpoint['id']+'-dns.jsonl.gz')))
                report['exhaustive'].append(dict(endpoint=endpoint['id'],**row));save(output/'report.json',report)
                print(endpoint['id'],'full corpus',row['queries'],flush=True)
        report['routes']=json.loads(subprocess.check_output(['ip','-n',NS,'-j','route'],text=True))
        if any(r['dst']=='default' for r in report['routes']):raise ValueError('upstream namespace has default route')
        report['qdisc']=subprocess.check_output(['ip','netns','exec',NS,'tc','-s','qdisc','show','dev',PEER],text=True)
        report['all_passed']=True
    except Exception as error:
        report['error']=f'{type(error).__name__}: {error}'
        raise
    finally:
        report['elapsed_s']=time.time()-began;save(output/'report.json',report)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',type=pathlib.Path,default=ROOT);p.add_argument('--output',type=pathlib.Path,required=True);p.add_argument('--full',action='store_true');a=p.parse_args()
    check(a.root,a.output,a.full)
