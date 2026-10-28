import pathlib
import tempfile
import unittest
try:
    import dns.message, dns.rrset, dns.zone
except ImportError:
    dns=None
from fixture import generate
from check import validate

@unittest.skipIf(dns is None,'requires installed dnspython client')
class CheckTests(unittest.TestCase):
    def test_rejects_wrong_answer_ttl_question_and_missing_negative_soa(self):
        with tempfile.TemporaryDirectory() as directory:
            folder=pathlib.Path(directory)/'fixture'
            generate(folder,{'domestic':[{'addr':'udp://one'}]},['a.example'])
            zone=dns.zone.from_file(str(folder/'zone-domestic.db'),origin='.',relativize=False)
            q=dns.message.make_query('a.fixture.test.','A');r=dns.message.make_response(q)
            r.answer=[dns.rrset.from_text('a.fixture.test.',300,'IN','A','192.0.2.10')]
            validate(r,q,zone,'udp')
            r.answer[0].ttl=299
            with self.assertRaisesRegex(ValueError,'answer/TTL/order'):validate(r,q,zone,'udp')
            r.answer=[dns.rrset.from_text('a.fixture.test.',300,'IN','A','203.0.113.10')]
            with self.assertRaises(ValueError):validate(r,q,zone,'udp')
            q=dns.message.make_query('absent.fixture.test.','A');r=dns.message.make_response(q);r.set_rcode(3)
            with self.assertRaisesRegex(ValueError,'SOA'):validate(r,q,zone,'udp')
