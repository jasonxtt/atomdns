import unittest

import flags


class Mapping(unittest.TestCase):
    def test_defaults_are_no_differences(self):
        self.assertEqual(flags.differences(flags.switch_letters({})), {})

    def test_reverse_switch9_and_split_switch1(self):
        letters = flags.switch_letters({9: "B", 1: "B"})
        self.assertEqual(flags.differences(letters), {"cn_fakeip": True, "block_empty": False, "block_blocklist": False})

    def test_client_routing(self):
        for two, twelve, expected in (("A", "B", "only_listed_proxy"), ("B", "A", "listed_direct"), ("A", "A", "none"), ("B", "B", "none")):
            self.assertEqual(flags.values(flags.switch_letters({2: two, 12: twelve}))["client_routing"], expected)

    def test_every_single_flip_changes_its_flags_only(self):
        for name, combination in flags.COMBINATIONS.items():
            if name.startswith("S") and "after_switches" not in combination:
                number = next(iter(combination["switches"]))
                keys = set(flags.differences(flags.switch_letters(combination["switches"])))
                expected = {k for k, entry in flags.TABLE.items() if entry[0] == number}
                self.assertEqual(keys, expected, name)

    def test_common_combination(self):
        self.assertEqual(flags.differences(flags.switch_letters(flags.COMBINATIONS["G1-common"]["switches"])),
                         {"core_mode": "safe", "client_routing": "listed_direct", "ddns_foreign": True})


if __name__ == "__main__":
    unittest.main()
