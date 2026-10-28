"""Static oracle eligibility is a config capability, not saved-state size."""
import unittest
from evaluate import static_oracle_block_reason


class StaticOracle(unittest.TestCase):
    def test_configured_learner_blocks_without_any_saved_rows(self):
        self.assertIsNotNone(static_oracle_block_reason({'runtime_features': {'route_learner': True}}))

    def test_unknown_legacy_manifest_is_not_treated_as_stateless(self):
        self.assertIsNotNone(static_oracle_block_reason({}))

    def test_verified_absence_can_use_static_oracle(self):
        self.assertIsNone(static_oracle_block_reason({'runtime_features': {'route_learner': False}}))


if __name__ == '__main__':
    unittest.main()
