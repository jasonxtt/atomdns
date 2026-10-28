#!/usr/bin/env python3
"""Install pinned official CoreDNS within an explicit private test root."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tarfile
import urllib.request

from fixture import VERSION, sha, save

ARCHIVE_SHA256='2495c5b8f3fab6d0a336c397f1227b75406d86e6ec7125744a29df9e936ab8cd'


def validate_plugins(plugins):
    names={line.removeprefix('dns.') for line in plugins.splitlines()}
    for name in ['file','template','tls','https','quic','prometheus']:
        if name not in names:raise ValueError('missing plugin '+name)


def install(root):
    root.mkdir(mode=0o700,parents=True,exist_ok=True)
    (root/'downloads').mkdir(exist_ok=True);(root/'bin').mkdir(exist_ok=True)
    name=f'coredns_{VERSION}_linux_amd64.tgz';archive=root/'downloads'/name
    if not archive.exists():
        urllib.request.urlretrieve(f'https://github.com/coredns/coredns/releases/download/v{VERSION}/{name}',archive)
    if sha(archive)!=ARCHIVE_SHA256:raise ValueError('official archive SHA256 mismatch')
    binary=root/'bin/coredns'
    if (root/'install.json').exists():
        old=json.loads((root/'install.json').read_text())
        if old['archive_sha256']!=ARCHIVE_SHA256 or sha(binary)!=old['binary_sha256']:raise ValueError('existing installation identity differs')
        return old
    with tarfile.open(archive) as source:
        item=source.getmember('coredns')
        if not item.isfile():raise ValueError('archive binary is not regular')
        payload=source.extractfile(item).read()
        if binary.exists():
            if sha(binary)!=hashlib.sha256(payload).hexdigest():raise ValueError('unmanaged binary differs from pinned archive')
        else:binary.write_bytes(payload)
    binary.chmod(0o755)
    version=subprocess.check_output([binary,'-version'],text=True).strip()
    if not version.startswith('CoreDNS-'+VERSION):raise ValueError('binary version mismatch')
    plugins=subprocess.check_output([binary,'-plugins'],text=True)
    validate_plugins(plugins)
    receipt=dict(version=version,archive_sha256=ARCHIVE_SHA256,binary_sha256=sha(binary),plugins=plugins.splitlines())
    save(root/'install.json',receipt)
    return receipt


def certificates(root):
    folder=root/'certs'
    if folder.exists():raise ValueError('certificate directory exists; explicit rotation required')
    folder.mkdir(mode=0o700)
    def run(args):subprocess.run(['openssl']+list(map(str,args)),check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    run(['req','-x509','-newkey','rsa:2048','-nodes','-keyout',folder/'ca-key.pem','-out',folder/'ca.pem','-days','3650','-subj','/CN=rtdns-isolated-test-CA','-addext','basicConstraints=critical,CA:TRUE','-addext','keyUsage=critical,keyCertSign,cRLSign'])
    run(['req','-newkey','rsa:2048','-nodes','-keyout',folder/'key.pem','-out',folder/'leaf.csr','-subj','/CN=localhost'])
    (folder/'leaf.ext').write_text('subjectAltName=DNS:localhost,IP:198.51.100.2\nbasicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n')
    run(['x509','-req','-in',folder/'leaf.csr','-CA',folder/'ca.pem','-CAkey',folder/'ca-key.pem','-CAcreateserial','-out',folder/'cert.pem','-days','365','-extfile',folder/'leaf.ext'])
    for p in folder.iterdir():p.chmod(0o600)
    save(folder/'identity.json',dict(ca_sha256=sha(folder/'ca.pem'),certificate_sha256=sha(folder/'cert.pem')))

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--root',type=pathlib.Path,default=pathlib.Path('/opt/rtdns-test-upstream'));p.add_argument('--certificates',action='store_true');a=p.parse_args()
    print(json.dumps(install(a.root)))
    if a.certificates:certificates(a.root)
