import unittest
from install import validate_plugins

class InstallTests(unittest.TestCase):
    def test_checks_real_plugin_names_and_missing_required_transport(self):
        validate_plugins('file\ntemplate\ntls\nhttps\nquic\nprometheus\n')
        with self.assertRaisesRegex(ValueError,'quic'):
            validate_plugins('file\ntemplate\ntls\nhttps\nprometheus\n')
