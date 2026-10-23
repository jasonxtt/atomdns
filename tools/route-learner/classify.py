"""Row comparison and verdicts of the route-learner differential (pure; unit-tested)."""
import collections

FIELDS = ("domain_set", "effective_tag", "final_sequence", "final_upstream_group")
CATEGORY_TAGS = ("my_sv4list", "my_realiplist", "my_fakeiplist", "my_nov6list", "my_nov4list")


def records(value):
    return sorted(tuple(row[:4]) for row in value.get("answers", []))


def split_joined(text):
    return [part.strip() for part in (text or "").split("|") if part.strip()]


def source_relation(a, b):
    """equal | order_only | different for two joined `matched_rule_source` strings."""
    left, right = split_joined(a), split_joined(b)
    if left == right:
        return "equal"
    return "order_only" if sorted(left) == sorted(right) else "different"


def row_differences(before, after, group_of):
    """Field names that differ between two replay rows of the same (pass, index).

    `group_of(audit)` maps an audit row to its upstream group name (as e2e-1's compare does)."""
    changed = []
    for row in (before, after):
        if "error" in row["dns"]:
            changed.append("dns_error")
        if row.get("audit_expected") and row["audit"] is None:
            changed.append("audit_missing")
    if before["dns"].get("rcode") != after["dns"].get("rcode"):
        changed.append("rcode")
    if records(before["dns"]) != records(after["dns"]):
        changed.append("answers")
    if before["audit"] is not None and after["audit"] is not None:
        a, b = dict(before["audit"]), dict(after["audit"])
        a["final_upstream_group"], b["final_upstream_group"] = group_of(a), group_of(b)
        for field in FIELDS:
            if (a.get(field) or "") != (b.get(field) or ""):
                changed.append(field)
    return sorted(set(changed))


LEARNED_SOURCES = frozenset({"my_sv4rule", "my_realiprule", "my_fakeiprule", "my_nov6rule", "my_nov4rule"})


def strip_sources(text, ignored):
    """Drops the ignored names and the `inline:` identity the audit shows when a mapper hit named no source."""
    return "|".join(part for part in split_joined(text) if part not in ignored and not part.startswith("inline:"))


def compare_rows(before_rows, after_rows, group_of, ignored_sources=frozenset()):
    """Per-row comparison. Returns (failures, source_counts, rows_compared); a failure is
    {"pass","index","name","type","fields"}; `matched_rule_source` is a diagnostic.

    `ignored_sources`: source names the OLD configuration only shows once its published rule file is loaded
    (a cold BEFORE has learned names in the in-memory overlay only, which has no source, so the audit shows the
    sequence rule's `inline:` identity) while the learner always names them; a row that differs only by them is counted as `learner_source_only`; the labels in `domain_set`
    (compared strictly) say which learned categories a name is in."""
    if len(before_rows) != len(after_rows):
        raise ValueError("replays have different lengths")
    failures, sources = [], collections.Counter()
    for a, b in zip(before_rows, after_rows):
        if (a["pass"], a["index"], a["input"]) != (b["pass"], b["index"], b["input"]):
            raise ValueError("replay order/input mismatch")
        fields = row_differences(a, b, group_of)
        if a["audit"] is not None and b["audit"] is not None:
            left, right = a["audit"].get("matched_rule_source"), b["audit"].get("matched_rule_source")
            relation = source_relation(left, right)
            if ignored_sources and relation == "different":
                # Learned sources are not compared for presence (BEFORE shows one only for names whose rule file
                # was already published; the label in `domain_set` is compared strictly instead); everything
                # else must agree as a set.
                stripped = source_relation(strip_sources(left, ignored_sources), strip_sources(right, ignored_sources))
                if stripped != "different":
                    relation = "learner_source_only"
            sources[relation] += 1
            if relation == "different":
                fields = sorted(set(fields) | {"matched_rule_source"})
        if fields:
            failures.append({"pass": a["pass"], "index": a["index"], "name": a["input"]["name"],
                             "type": a["input"]["type"], "fields": fields})
    return failures, dict(sources), len(before_rows)


def set_differences(before, after):
    """{category: {"only_before": [...], "only_after": [...]}} for categories that differ."""
    result = {}
    for tag in CATEGORY_TAGS:
        left, right = set(before.get(tag, ())), set(after.get(tag, ()))
        if left != right:
            result[tag] = {"only_before": sorted(left - right), "only_after": sorted(right - left)}
    return result


def servfail_allowance(sets_diff):
    """Names whose learning legitimately differs when the upstream answered SERVFAIL: the old
    configuration learned them as no-V4/no-V6, the new one does not (and never the reverse)."""
    allowed = set()
    for tag in ("my_nov4list", "my_nov6list"):
        entry = sets_diff.get(tag)
        if entry:
            allowed |= set(entry["only_before"])
    unexpected = {tag: entry for tag, entry in sets_diff.items()
                  if tag not in ("my_nov4list", "my_nov6list") or entry["only_after"]}
    return allowed, unexpected


HARD_FAILURES = frozenset({"dns_error", "audit_missing", "upstream_mapping"})


def servfail_split(failures, allowed):
    """(explained, unexplained) failures of group E: a row is explained when its name is in `allowed` and it
    differs only in routing/answer fields; a DNS error or a missing audit row is never explained."""
    explained = [f for f in failures if f["name"] in allowed and not set(f["fields"]) & HARD_FAILURES]
    unexplained = [f for f in failures if not (f["name"] in allowed and not set(f["fields"]) & HARD_FAILURES)]
    return explained, unexplained
