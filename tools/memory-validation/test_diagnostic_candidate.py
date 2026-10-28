"""Diagnostic transforms must remain private and advertise changed hashing."""
import hashlib
import json
import pathlib
import tempfile
import unittest
from diagnostic_candidate import create


class DiagnosticCandidate(unittest.TestCase):
    def test_synthetic_copy_is_explicit_and_never_mutates_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=pathlib.Path(temporary); source=root/'source'; output=root/'candidate'
            observer=source/'rust/native-host/src/observer.rs'
            retention=observer.parent/'observer/retention.rs'
            retention.parent.mkdir(parents=True)
            observer.write_text('    audit_timings: VecDeque<T>,\n    fn observation(elapsed: Duration) -> TerminalObservation {\n}\n')
            original='use std::collections::hash_map::RandomState;\nlet hash = RandomState::new();\n'
            retention.write_text(original)
            probe=source/'tools/5d-1/audit_memory_compact.rs';probe.parent.mkdir(parents=True);probe.write_text('// accounting fixture\n')
            create(source,output,synthetic=True)
            manifest=json.loads((output/'memory-diagnostic.json').read_text())
            self.assertTrue(manifest['excluded_from_timing_acceptance'])
            self.assertEqual(manifest['sharing_hash'],'fixed diagnostic DefaultHasher')
            self.assertEqual(retention.read_text(),original)
            self.assertNotIn('include!',observer.read_text())
            fixture=output/'rust/native-host/src/observer/memory_synthetic_fixture.rs'
            self.assertEqual(hashlib.sha256(fixture.read_bytes()).hexdigest(),manifest['fixture_sha256'])
            with self.assertRaises(ValueError):create(source,output,synthetic=True)


if __name__=='__main__':unittest.main()
