import json
import ipaddress
import pathlib
import tempfile
import unittest

from fixture import generate, map_entries


class FixtureTests(unittest.TestCase):
    def test_generated_address_records_use_documentation_ranges(self):
        networks=[ipaddress.ip_network(n) for n in ['192.0.2.0/24','198.51.100.0/24','203.0.113.0/24','2001:db8::/32']]
        groups={g:[{'addr':'udp://supplier.example'}] for g in ['domestic','cnfake','nocnfake','foreign']}
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)/'fixture'
            generate(root,groups,['a.example','b.example'])
            for path in root.glob('zone-*.db'):
                for line in path.read_text().splitlines():
                    if ' IN A ' not in line and ' IN AAAA ' not in line:continue
                    address=ipaddress.ip_address(line.split()[-1])
                    self.assertTrue(any(address.version==n.version and address in n for n in networks),line)
                    if not line.startswith('ns.fixture.test.'):
                        self.assertNotIn(address,ipaddress.ip_network('198.51.100.0/24'))

    def test_preserves_each_enabled_supplier_identity_and_protocol(self):
        groups={'domestic':[{'tag':'a','protocol':'udp','addr':'udp://one'}, {'tag':'b','protocol':'tls','addr':'tls://two'}, {'tag':'off','enabled':False,'addr':'udp://off'}]}
        values=map_entries(groups)
        self.assertEqual([v['tag'] for v in values],['a','b'])
        self.assertNotEqual(values[0]['ports'],values[1]['ports'])
        self.assertEqual([v['delay_ms'] for v in values],[0,30])
        self.assertEqual(values[1]['protocol'],'tls')
        with self.assertRaisesRegex(ValueError,'protocol'):
            map_entries({'domestic':[{'tag':'x','protocol':'surprise','addr':'whatever'}]})

    def test_repeatable_complete_fixture_with_complexity_and_no_external_egress(self):
        groups={'domestic':[{'tag':'a','addr':'udp://one'}]}
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)
            a=generate(root/'a',groups,['a.example','b.example','a.example'])
            b=generate(root/'b',groups,['a.example','b.example','a.example'])
            self.assertEqual(a['content_sha256'],b['content_sha256'])
            text=(root/'a/Corefile').read_text()
            self.assertNotIn('forward ',text)
            self.assertNotIn('cache ',text)
            self.assertNotIn('loadbalance',text)
            self.assertIn('reload 0',text)
            self.assertEqual(a['names'],2)
            self.assertTrue(a['records_sha256'])
            zone=(root/'a/zone-domestic.db').read_text()
            for token in [' IN AAAA ', ' IN CNAME ', ' IN TXT ', ' IN MX ', ' IN SRV ']:
                self.assertIn(token,zone)
            self.assertIn('rcode SERVFAIL',text)
            with self.assertRaises(FileExistsError):generate(root/'a',groups,['a.example'])

    def test_rejects_empty_invalid_and_unsupported_data(self):
        with tempfile.TemporaryDirectory() as directory:
            for names in [[],['bad..example'],['a\nmalicious.example']]:
                with self.assertRaises(ValueError):generate(pathlib.Path(directory)/'new',{'domestic':[{'addr':'udp://one'}]},names)
