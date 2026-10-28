import json
import pathlib
import tempfile
import unittest
from proxy import allowed_target
from fixture import IP,map_entries
from prepare_case import remap_settings

class PrepareTests(unittest.TestCase):
    def test_identity_proxy_flags_disabled_entries_and_ports_are_preserved(self):
        groups={'foreign':[dict(tag='a',addr='https://a/query',use_proxy=True),dict(tag='b',addr='tls://b',enabled=False)]}
        settings=dict(upstreams=groups,proxies={'main_proxy':{'addr':'original'}})
        values=map_entries(groups);new=remap_settings(settings,groups,values)
        self.assertEqual(new['upstreams']['foreign'][0]['tag'],'a')
        self.assertTrue(new['upstreams']['foreign'][0]['use_proxy'])
        self.assertTrue(new['upstreams']['foreign'][0]['enabled'])
        self.assertEqual(new['upstreams']['foreign'][0]['dial_addr'],IP+':20002')
        self.assertEqual(new['upstreams']['foreign'][1],groups['foreign'][1])
        self.assertEqual(settings['proxies']['main_proxy']['addr'],'original')
        self.assertTrue(allowed_target(IP,20002,{20002}))
        self.assertFalse(allowed_target('192.0.2.53',20002,{20002}))
        self.assertFalse(allowed_target(IP,22,{20002}))


    def test_single_upstream_profile_keeps_first_enabled_per_business_group(self):
        groups={'domestic':[dict(tag='off',addr='udp://off',enabled=False),dict(tag='a',addr='udp://a'),dict(tag='b',addr='udp://b')],
                'foreign':[dict(tag='x',addr='https://x/query',use_proxy=True),dict(tag='y',addr='https://y/query')]}
        values=map_entries(groups);settings=dict(upstreams=groups)
        out=remap_settings(settings,groups,values,single_upstream=True)
        self.assertEqual({g:[e['tag'] for e in es] for g,es in out['upstreams'].items()},{'domestic':['a'],'foreign':['x']})
        self.assertTrue(out['upstreams']['foreign'][0]['use_proxy'])
        self.assertEqual(len(settings['upstreams']['domestic']),3)
