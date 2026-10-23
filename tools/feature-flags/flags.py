"""The switch-to-flag mapping of M2-1 and the combinations the differential runs (pure; unit-tested).

This is an independent copy of the program's migration table (`rust/native-host/src/config/flags.rs`), so the
differential checks the program's migration instead of trusting it."""

# Package defaults of the schema-2 `config_rust` (`rule/switchN.txt`).
DEFAULTS = {1: "A", 2: "B", 3: "A", 4: "A", 5: "A", 6: "B", 7: "A", 8: "B", 9: "A", 12: "B", 13: "A", 16: "B", 17: "A"}
ALL_SWITCHES = range(1, 18)

# key: (switch, value for A, value for B, default)
TABLE = {
    "core_mode": (3, "compat", "safe", "compat"),
    "dns_mode": (17, "fakeip", "realip", "fakeip"),
    "cn_fakeip": (9, False, True, False),
    "prefer_v4": (8, True, False, False),
    "ddns_foreign": (16, True, False, False),
    "block_empty": (1, True, False, True),
    "block_blocklist": (1, True, False, True),
    "block_qtypes": (5, True, False, True),
    "block_ads": (7, True, False, True),
    "block_aaaa": (6, True, False, False),
    "lazy_cache": (4, True, False, True),
    "full_cache": (13, True, False, True),
}


def switch_letters(changes):
    """The package defaults with `changes` ({switch: "A"|"B"}) applied."""
    letters = dict(DEFAULTS)
    letters.update(changes)
    return letters


def values(letters):
    """Every flag's value for a full set of switch letters."""
    result = {key: (a if letters[number] == "A" else b) for key, (number, a, b, _) in TABLE.items()}
    pair = (letters[2], letters[12])
    result["client_routing"] = {("A", "B"): "only_listed_proxy", ("B", "A"): "listed_direct"}.get(pair, "none")
    return result


def differences(letters):
    """The `flags` section for `letters`: the values that differ from the defaults."""
    defaults = {key: entry[3] for key, entry in TABLE.items()}
    defaults["client_routing"] = "none"
    return {key: value for key, value in values(letters).items() if value != defaults[key]}


def _flip(number):
    return {number: "B" if DEFAULTS[number] == "A" else "A"}


# `after_switches` (negative controls only) gives AFTER other values than BEFORE.
# mode: "migrate" (AFTER gets the switch files and migrates them at start), "direct" (AFTER gets `flags` in
# settings.json), "hot" (both start at the defaults and change through their APIs after the first pass).
COMBINATIONS = {
    "G0-defaults": {"mode": "migrate", "switches": {}},
    "G1-common": {"mode": "direct", "switches": {3: "B", 2: "B", 12: "A", 16: "A"}},
    "S3-core_mode": {"mode": "migrate", "switches": _flip(3)},
    "S17-dns_mode": {"mode": "direct", "switches": _flip(17)},
    "S9-cn_fakeip": {"mode": "migrate", "switches": _flip(9)},
    "S8-prefer_v4": {"mode": "direct", "switches": _flip(8)},
    "S16-ddns_foreign": {"mode": "migrate", "switches": _flip(16)},
    "S1-block_empty+blocklist": {"mode": "direct", "switches": _flip(1)},
    "S5-block_qtypes": {"mode": "migrate", "switches": _flip(5)},
    "S7-block_ads": {"mode": "direct", "switches": _flip(7)},
    "S6-block_aaaa": {"mode": "migrate", "switches": _flip(6)},
    "S4-lazy_cache": {"mode": "direct", "switches": _flip(4)},
    "S13-full_cache": {"mode": "migrate", "switches": _flip(13)},
    "C1-only_listed_proxy": {"mode": "direct", "switches": {2: "A", 12: "B"}},
    "C2-listed_direct": {"mode": "migrate", "switches": {2: "B", 12: "A"}},
    "C3-none_from_AA": {"mode": "migrate", "switches": {2: "A", 12: "A"}},
    "H-hot_common": {"mode": "hot", "switches": {3: "B", 2: "B", 12: "A", 16: "A"}},
    # Negative controls: AFTER is deliberately set differently; these must show differing rows.
    "N1-control_core_mode": {"mode": "direct", "switches": {}, "after_switches": {3: "B"}},
    "N2-control_client_routing": {"mode": "direct", "switches": {}, "after_switches": {2: "B", 12: "A"}},
}
