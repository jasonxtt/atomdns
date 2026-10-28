#!/usr/bin/env python3
"""Copy the frozen production root and remap each supplier to its own fixture."""
import copy
import json
import pathlib
import shutil
import sys
from fixture import IP,save,sha

HERE=pathlib.Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'memory-validation'))
from real_config_probe import prepare as copy_config,effective_upstreams,documents


def remap_settings(settings,groups,endpoints,single_upstream=False):
    value=copy.deepcopy(settings)
    value.setdefault('proxies',{}).setdefault('main_proxy',{})['addr']='127.0.0.1:17891'
    for group,entries in groups.items():
        targets=[e for e in endpoints if e['group']==group];at=0;result=[]
        for original in entries:
            e=copy.deepcopy(original)
            if e.get('enabled',True):
                if at>=len(targets):raise ValueError('missing provider endpoint')
                target=targets[at];at+=1
                if target['tag']!=e.get('tag',''):raise ValueError('supplier identity mismatch')
                protocol=target['protocol'];port=target['ports'][protocol]
                host='localhost' if protocol in ['tls','https','quic'] else IP
                e['enabled']=True;e['protocol']=protocol;e['addr']=f'{protocol}://{host}:{port}'+('/dns-query' if protocol=='https' else '')
                e.pop('bootstrap',None)
                if protocol in ['tls','https','quic']:e['dial_addr']=IP+':'+str(port)
                else:e.pop('dial_addr',None)
            result.append(e)
        if at!=len(targets):raise ValueError('extra provider endpoint')
        value.setdefault('upstreams',{})[group]=[next(e for e in result if e.get('enabled',True))] if single_upstream else result
    return value


def prepare(source,output,fixture,certs,single_upstream=False):
    manifest=copy_config(source,output)
    groups=effective_upstreams(source)['upstreams']
    settings=json.loads((output/'webinfo/settings.json').read_text())
    mapped=remap_settings(settings,groups,fixture['endpoints'],single_upstream);save(output/'webinfo/settings.json',mapped)
    for path,doc in documents(output):
        for d in doc.get('plugins',[]):
            if d['type']=='socks5_proxy':d['args']['addr']='127.0.0.1:17891'
        import yaml
        path.write_text(yaml.safe_dump(doc,sort_keys=False,allow_unicode=True))
    shutil.copyfile(certs/'ca.pem',output/'ca.pem')
    (output/'empty-certs').mkdir();(output/'cache').mkdir(exist_ok=True);(output/'tmp').mkdir()
    if list((output/'cache').glob('*.dump')):raise ValueError('unexpected warm-cache dump in frozen source')
    manifest.update(mode='offline CoreDNS individual supplier endpoints',profile='single-per-group' if single_upstream else 'original-provider-count',fixture=fixture,
                    prepared={str(p.relative_to(output)):sha(p) for p in output.rglob('*') if p.is_file()})
    return manifest
