#!/usr/bin/env python3
"""BASE/BASE supplier/state qualification before any timing trial."""
import argparse
import json
import pathlib
import random
import sys
from fixture import save,sha
from service import ROOT,verify_fixture
from prepare_case import prepare
from case import Case
HERE=pathlib.Path(__file__).resolve().parent
sys.path.insert(0,str(HERE.parent/'memory-validation'))
from state_model import compare,state
from real_config_probe import documents
from load import corpus


def run(args):
    args.output.mkdir(mode=0o700,parents=True)
    folder,fixture=verify_fixture(args.upstream_root)
    work=args.output/'work';work.mkdir();save(work/'fixture-manifest.json',fixture)
    manifest=prepare(args.source,work/'native',fixture,args.upstream_root/'certs',args.single_upstream);save(work/'input-manifest.json',manifest)
    tags=[d['tag'] for _,doc in documents(work/'native') for d in doc.get('plugins',[]) if d['type']=='route_learner']
    values=corpus((work/'native/gen/top_domains.txt').read_text())
    indices=list(range(len(values))) if args.limit is None else sorted(set(list(range(args.limit))+[i for i,v in enumerate(values) if v['kind']!='production']))
    random.Random(501).shuffle(indices)
    seq=[dict(values[i],corpus_index=i) for _ in range(2) for i in indices]
    save(args.output/'sequence.json',seq)
    report=dict(scope='closed-loop BASE/control DNS+learner+route qualification; not performance',all_passed=False,planned_per_role=len(seq),
                profile=manifest['profile'],full_corpus=args.limit is None,whole_project_accepted=False,binary_sha256=sha(args.base),fixture_sha256=sha(folder/'manifest.json'),roles={})
    try:
        for role in ['base','control']:
            print(role,'starting',len(seq),flush=True)
            with Case(work,args.base,args.output/role,args.upstream_root) as env:
                spec=dict(api_port=49099,dns_port=40053,ttl_ceiling=600,learners=tags,sequence=str((args.output/'sequence.json').resolve()),output=str((args.output/role).resolve()))
                save(args.output/role/'worker-spec.json',spec)
                child=env.spawn(1,['python3',HERE.parent/'memory-validation/state_replay.py','--worker',args.output/role/'worker-spec.json'],'worker.log')
                if child.wait()!=0:raise RuntimeError('functional worker failed: '+role)
            report['roles'][role]=dict(cleanup=json.loads((args.output/role/'cleanup.json').read_text()),existing_identity=json.loads((args.output/role/'existing-service-guard.json').read_text()))
            save(args.output/'report.json',report)
        def rows(role):return [json.loads(line) for line in (args.output/role/'trace.jsonl').read_text().splitlines()]
        def initial(role):return json.loads((args.output/role/'initial-learner.json').read_text())
        result=compare(rows('base'),rows('control'),len(seq),initial('base'),initial('control'))
        final_equal=state(json.loads((args.output/'base/final-learner.json').read_text()))==state(json.loads((args.output/'control/final-learner.json').read_text()))
        report.update(comparison=result,final_learner_equal=final_equal,all_passed=result['status']=='pass' and final_equal)
        print('qualification',result['status'],'final_equal',final_equal,flush=True)
    except Exception as error:
        report['error']=f'{type(error).__name__}: {error}'
        raise
    finally:save(args.output/'report.json',report)

if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--upstream-root',type=pathlib.Path,default=ROOT);p.add_argument('--source',type=pathlib.Path,required=True);p.add_argument('--base',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);p.add_argument('--limit',type=int);p.add_argument('--single-upstream',action='store_true');a=p.parse_args();run(a)
