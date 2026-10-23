import json
import pathlib
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import classify  # noqa: E402
import gen_to_store  # noqa: E402


def row(passed, index, rcode=0, answers=(), audit=None, name="a.example.com", qtype=1):
    return {"pass": passed, "index": index, "input": {"name": name, "type": qtype, "port": 53},
            "dns": {"rcode": rcode, "answers": [list(a) for a in answers]}, "audit": audit, "audit_expected": audit is not None}


def audit(**fields):
    base = {"domain_set": "x", "effective_tag": "x", "final_sequence": "s", "final_upstream": "u", "matched_rule_source": "r"}
    base.update(fields)
    return base


class Converter(unittest.TestCase):
    def test_lists_and_rules_are_merged_normalised_and_deduplicated(self):
        with tempfile.TemporaryDirectory() as tmp:
            gen = pathlib.Path(tmp)
            (gen / "realiplist.txt").write_text("0000000003 2026-01-01 Example.COM.\n0000000001 a.example.org|AD\njunk\n")
            (gen / "realiprule.txt").write_text("full:example.com\nfull:Other.Example.NET\nregexp:x\n")
            (gen / "nov6list.txt").write_text("1 2026-01-01 v6.example.net\n")
            out = pathlib.Path(tmp) / "out/route_learner.json"
            counts = gen_to_store.convert(gen, out, now=1000)
            self.assertEqual(counts["my_realiplist"], 3)
            self.assertEqual(counts["my_nov6list"], 1)
            self.assertEqual(counts["my_fakeiplist"], 0)
            data = json.loads(out.read_text())
            self.assertEqual(data["format"], 1)
            self.assertEqual([r[0] for r in data["categories"]["my_realiplist"]], ["a.example.org", "example.com", "other.example.net"])
            self.assertEqual(data["categories"]["my_nov6list"][0][1:4], [1000, 1000, 0])


class Classify(unittest.TestCase):
    group = staticmethod(lambda audit: audit.get("final_upstream", ""))

    def test_equal_rows_pass_and_every_audited_field_counts(self):
        a = [row(1, 0, audit=audit())]
        self.assertEqual(classify.compare_rows(a, [row(1, 0, audit=audit())], self.group)[0], [])
        for field in classify.FIELDS[:3]:
            failures = classify.compare_rows(a, [row(1, 0, audit=audit(**{field: "changed"}))], self.group)[0]
            self.assertEqual(failures[0]["fields"], [field])
        failures = classify.compare_rows(a, [row(1, 0, audit=audit(final_upstream="other"))], self.group)[0]
        self.assertEqual(failures[0]["fields"], ["final_upstream_group"])

    def test_rcode_answers_and_errors(self):
        base = [row(1, 0, answers=[("a.", 1, 1, "192.0.2.1", 5)])]
        self.assertEqual(classify.compare_rows(base, [row(1, 0, answers=[("a.", 1, 1, "192.0.2.1", 9)])], self.group)[0], [])
        self.assertEqual(classify.compare_rows(base, [row(1, 0, answers=[("a.", 1, 1, "192.0.2.2", 5)])], self.group)[0][0]["fields"], ["answers"])
        self.assertEqual(classify.compare_rows(base, [row(1, 0, rcode=2, answers=[("a.", 1, 1, "192.0.2.1", 5)])], self.group)[0][0]["fields"], ["rcode"])
        broken = row(1, 0)
        broken["dns"] = {"error": "timeout"}
        self.assertIn("dns_error", classify.compare_rows(base, [broken], self.group)[0][0]["fields"])

    def test_source_order_is_a_diagnostic_but_a_different_set_fails(self):
        a = [row(1, 0, audit=audit(matched_rule_source="x|y"))]
        failures, sources, _ = classify.compare_rows(a, [row(1, 0, audit=audit(matched_rule_source="y|x"))], self.group)
        self.assertEqual((failures, sources), ([], {"order_only": 1}))
        failures, sources, _ = classify.compare_rows(a, [row(1, 0, audit=audit(matched_rule_source="x|z"))], self.group)
        self.assertEqual(failures[0]["fields"], ["matched_rule_source"])
        self.assertEqual(sources, {"different": 1})

    def test_a_learner_only_source_is_ignored_when_asked(self):
        a = [row(1, 0, audit=audit(matched_rule_source="white"))]
        b = [row(1, 0, audit=audit(matched_rule_source="my_realiprule|white"))]
        self.assertEqual(classify.compare_rows(a, b, self.group)[0][0]["fields"], ["matched_rule_source"])
        failures, sources, _ = classify.compare_rows(a, b, self.group, classify.LEARNED_SOURCES)
        self.assertEqual((failures, sources), ([], {"learner_source_only": 1}))
        # Same learned parts in another order is an order difference, not a presence difference.
        d = [row(1, 0, audit=audit(matched_rule_source="my_realiprule|white"))]
        e = [row(1, 0, audit=audit(matched_rule_source="white|my_realiprule"))]
        failures, sources, _ = classify.compare_rows(d, e, self.group, classify.LEARNED_SOURCES)
        self.assertEqual((failures, sources), ([], {"order_only": 1}))
        c = [row(1, 0, audit=audit(matched_rule_source="my_realiprule|black"))]
        self.assertEqual(classify.compare_rows(a, c, self.group, classify.LEARNED_SOURCES)[0][0]["fields"], ["matched_rule_source"])

    def test_mismatched_replays_are_refused(self):
        with self.assertRaises(ValueError):
            classify.compare_rows([row(1, 0)], [row(1, 1)], self.group)
        with self.assertRaises(ValueError):
            classify.compare_rows([row(1, 0)], [], self.group)

    def test_set_differences_and_the_servfail_allowance(self):
        before = {"my_nov6list": ["a", "b"], "my_realiplist": ["x"]}
        after = {"my_nov6list": ["a"], "my_realiplist": ["x"]}
        diff = classify.set_differences(before, after)
        self.assertEqual(diff, {"my_nov6list": {"only_before": ["b"], "only_after": []}})
        allowed, unexpected = classify.servfail_allowance(diff)
        self.assertEqual((allowed, unexpected), ({"b"}, {}))
        # A name only AFTER learned, or any other category differing, is unexpected.
        _, unexpected = classify.servfail_allowance(classify.set_differences({}, {"my_nov4list": ["z"]}))
        self.assertIn("my_nov4list", unexpected)
        _, unexpected = classify.servfail_allowance(classify.set_differences({"my_realiplist": ["q"]}, {}))
        self.assertIn("my_realiplist", unexpected)
        failures = [{"name": "b", "fields": ["rcode"]}, {"name": "c", "fields": ["answers"]}]
        explained, unexplained = classify.servfail_split(failures, {"b"})
        self.assertEqual(([f["name"] for f in explained], [f["name"] for f in unexplained]), (["b"], ["c"]))
        # A DNS error or a missing audit row of an allowed name is still a failure.
        hard = [{"name": "b", "fields": ["rcode", "dns_error"]}, {"name": "b", "fields": ["audit_missing"]}]
        self.assertEqual(classify.servfail_split(hard, {"b"}), ([], hard))


if __name__ == "__main__":
    unittest.main()
