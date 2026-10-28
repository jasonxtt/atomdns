import pathlib
import tempfile
import unittest
from service import verify_fixture, topology
from fixture import generate

class ServiceTests(unittest.TestCase):
    def test_tampered_fixture_rejects_before_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)
            generate(root/'fixtures/v1',{'domestic':[{'tag':'a','addr':'udp://one'}]},['a.example'])
            (root/'active').write_text('v1\n')
            self.assertEqual(verify_fixture(root)[1]['version'],'1.15.0')
            (root/'fixtures/v1/zone-domestic.db').write_text('tampered')
            with self.assertRaisesRegex(ValueError,'fingerprint'):verify_fixture(root)
            (root/'active').write_text('../escape')
            with self.assertRaisesRegex(ValueError,'fixture id'):verify_fixture(root)

    def test_network_commands_only_touch_owned_interfaces_and_namespace(self):
        plan=topology([dict(ports={'udp':20000,'tcp':20000,'tls':20001,'https':20002,'quic':20003},delay_ms=30)])
        text='\n'.join(' '.join(a) for a in plan)
        self.assertNotIn('eth0',text)
        self.assertNotIn('default',text)
        self.assertNotIn('sysctl',text)
        self.assertIn('rtup-br',text)
        self.assertIn('netem delay 30ms',text)
        self.assertIn('ip_proto udp src_port 20000',text)
        self.assertIn('ip_proto tcp src_port 20001',text)

    def test_readiness_retries_new_bound_socket_after_network_creation(self):
        from unittest.mock import MagicMock,patch
        from service import wait_ready
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory)
            generate(root/'fixtures/v1',{'domestic':[{'tag':'a','addr':'udp://one'}]},['a.example'])
            (root/'active').write_text('v1')
            first,second=MagicMock(),MagicMock()
            first.__enter__.return_value=first;second.__enter__.return_value=second
            first.bind.side_effect=OSError('interface not ready');first.recv.return_value=b''
            second.recv.return_value=bytes.fromhex('beef850000010001')
            with patch('service.socket.socket',side_effect=[first,second]) as factory,patch('service.time.sleep'),patch('service.time.monotonic',side_effect=[0,0,1,31]):
                wait_ready(root)
            self.assertEqual(factory.call_count,2)
            first.connect.assert_not_called()
            second.bind.assert_called_once_with(('198.51.100.1',0))
