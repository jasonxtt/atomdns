import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class LosslessToolsTest(unittest.TestCase):
    def test_prioritized_profile_only_wraps_timed_sender_and_preserves_sampler_priority(self):
        import ast
        module = load('two_cpu_profile')
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / 'derived'
            with patch.object(module.os, 'sched_getaffinity', return_value={0, 1}, create=True):
                receipt = module.derive(HERE.parent, output, prioritize_sender=True)
            case = next(n for n in ast.parse((output / 'test-upstream/case.py').read_text()).body
                        if isinstance(n, ast.ClassDef) and n.name == 'Case')
            spawn = next(n for n in case.body if isinstance(n, ast.FunctionDef) and n.name == 'spawn')
            class Owner:
                def spawn(self, cpu, argv, name):
                    return cpu, argv, name
            case.bases = [ast.Name(id='Owner', ctx=ast.Load())]
            case.body = [spawn]
            tree = ast.fix_missing_locations(ast.Module(body=[case], type_ignores=[]))
            scope = {'Owner': Owner}
            exec(compile(tree, '<owned-profile>', 'exec'), scope)
            instance = scope['Case']()
            for name in ['host.log', 'proxy.log', 'fill.log', 'post-reads.log']:
                self.assertEqual(instance.spawn(1, ['python3', 'worker.py'], name),
                                 (1, ['python3', 'worker.py'], name))
            self.assertEqual(instance.spawn(1, ['python3', 'worker.py'], 'load.log'),
                             (1, ['nice', '-n', '-5', 'python3', 'worker.py'], 'load.log'))
            self.assertEqual(receipt['nice']['sampler'], 10)
            self.assertEqual(receipt['nice']['sender'], -5)
            self.assertIn('os.nice(10 - os.getpriority(os.PRIO_PROCESS, 0))',
                          (output / '5d-2/fload.py').read_text())

    def test_post_read_validation_rejects_bad_rank_and_corrupt_dump(self):
        import gzip
        module = load('lossless_post_reads')
        module.validate_body('rank_domain', b'[{"key":"example","count":3}]')
        module.validate_body('cache_show_0', b'')
        module.validate_body('cache_dump_0', gzip.compress(b''))
        for name, body in [('rank_domain', b'{}'), ('rank_domain', b'[{"key":"x","count":true}]'),
                           ('cache_dump_0', b'not gzip'),
                           ('cache_dump_0', gzip.compress(b'\0' * 7)),
                           ('cache_show_0', b'<html>not the cache API</html>')]:
            with self.assertRaises((ValueError, OSError)):
                module.validate_body(name, body)

    def test_lock_reader_rejects_partial_or_unrecognized_identity_fields(self):
        module = load('lossless_probe')
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'Cargo.lock'
            for text in ['[[package]]\nname = "x"\n',
                         '[[package]]\nname = "x"\nversion = "1"\nsource = broken\n']:
                path.write_text(text)
                with self.assertRaises(ValueError):
                    module.lock_packages(path)

    def test_profile_refuses_unsupported_cpu_without_creating_output(self):
        module = load('two_cpu_profile')
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / 'output'
            with patch.object(module.os, 'sched_getaffinity', return_value={0}, create=True):
                with self.assertRaises(ValueError):
                    module.derive(HERE.parent, output)
            self.assertFalse(output.exists())

    def test_profile_keeps_source_intact_and_refuses_overwrite_or_source_drift(self):
        module = load('two_cpu_profile')
        with tempfile.TemporaryDirectory() as folder:
            output = Path(folder) / 'derived'
            files = ['test-upstream/service.py', 'test-upstream/case.py', 'test-upstream/performance.py']
            before = {name: (HERE.parent / name).read_bytes() for name in files}
            with patch.object(module.os, 'sched_getaffinity', return_value={0, 1}, create=True):
                receipt = module.derive(HERE.parent, output)
                self.assertEqual({name: (HERE.parent / name).read_bytes() for name in files}, before)
                self.assertNotEqual(receipt['original_sha256']['test-upstream/service.py'],
                                    receipt['derived_sha256']['test-upstream/service.py'])
                self.assertIn('upstream-readiness', (output / files[1]).read_text())
                with self.assertRaises(ValueError):
                    module.derive(HERE.parent, output)
                (output / files[0]).write_text('changed method')
                drift_output = Path(folder) / 'drift'
                with self.assertRaises(ValueError):
                    module.derive(output, drift_output)
                self.assertFalse(drift_output.exists())


if __name__ == '__main__':
    unittest.main()
