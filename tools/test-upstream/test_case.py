import subprocess
import unittest
from unittest.mock import patch
from case import cleanup_link

class CaseCleanup(unittest.TestCase):
    def test_namespace_destruction_may_remove_link_between_inspection_and_delete(self):
        link=dict(ifname='rts123',ifalias='atomdns-test-case')
        with patch('case.links',side_effect=[[link],[]]),patch('case.command',side_effect=subprocess.CalledProcessError(1,['ip'])):
            cleanup_link('rts123')

    def test_unowned_or_persistent_interface_errors_are_not_waived(self):
        with patch('case.links',return_value=[dict(ifname='rts123',ifalias='other')]),patch('case.command') as call:
            with self.assertRaises(RuntimeError):cleanup_link('rts123')
            call.assert_not_called()
        link=dict(ifname='rts123',ifalias='atomdns-test-case')
        with patch('case.links',return_value=[link]),patch('case.command',side_effect=subprocess.CalledProcessError(1,['ip'])):
            with self.assertRaises(subprocess.CalledProcessError):cleanup_link('rts123')
