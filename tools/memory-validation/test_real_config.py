"""Production comparison retains effective upstream/proxy bytes."""
import json
import pathlib
import tempfile
import socket
import unittest
from real_config_probe import prepare, probe_port
from state_replay import valid_ttls


class RealConfig(unittest.TestCase):
    def test_only_owned_listener_references_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root=pathlib.Path(directory);source=root/'source';source.mkdir();(source/'webinfo').mkdir()
            (source/'config_custom.yaml').write_text('api: {http: "0.0.0.0:9099"}\nplugins:\n- {tag: dns, type: udp_server, args: {listen: ":53"}}\n- {tag: rewrite, type: rewrite, args: {dns: "127.0.0.1:53"}}\n- {tag: domestic, type: aliapi, args: {upstreams: [{addr: "127.0.0.1:5533"}]}}\n')
            settings=json.dumps(dict(upstreams=dict(domestic=[dict(addr='192.0.2.53',tag='real')]),proxies=dict(main_proxy=dict(addr='192.0.2.1:7890'))))
            (source/'webinfo/settings.json').write_text(settings)
            output=root/'output';prepare(source,output)
            self.assertEqual((output/'webinfo/settings.json').read_text(),settings)
            import yaml
            config=yaml.safe_load((output/'config_custom.yaml').read_text())
            self.assertEqual(config['plugins'][0]['args']['listen'],'127.0.0.1:40053')
            self.assertEqual(config['plugins'][1]['args']['dns'],'127.0.0.1:40053')
            self.assertEqual(config['plugins'][2]['args']['upstreams'][0]['addr'],'127.0.0.1:5533')
            with self.assertRaises(ValueError):prepare(source,output)

    def test_real_observation_keeps_legal_ttls_without_changing_mock_default(self):
        dns=dict(answers=[('name',1,1,'data',86400)],authority=[],additional=[])
        self.assertFalse(valid_ttls(dns))
        self.assertTrue(valid_ttls(dns, 2**32-1))
        dns['answers'][0]=('name',1,1,'data',2**32)
        self.assertFalse(valid_ttls(dns, 2**32-1))

    def test_closed_tcp_time_wait_is_available_but_live_sockets_are_not(self):
        with socket.socket() as server:
            server.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1)
            server.bind(('127.0.0.1',0));server.listen()
            port=server.getsockname()[1]
            with self.assertRaises(OSError):probe_port(port,socket.SOCK_STREAM)
            with socket.socket() as client:
                client.connect(('127.0.0.1',port));accepted,_=server.accept()
                accepted.close();self.assertEqual(client.recv(1),b'')
        probe_port(port,socket.SOCK_STREAM)
        with socket.socket(socket.AF_INET,socket.SOCK_DGRAM) as udp:
            udp.bind(('127.0.0.1',0))
            with self.assertRaises(OSError):probe_port(udp.getsockname()[1],socket.SOCK_DGRAM)
