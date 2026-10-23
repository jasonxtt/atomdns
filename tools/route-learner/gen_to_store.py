"""Test-only: turn the former learned lists of a `gen/` directory into a `route_learner` state file.

The product does not import the old lists. This exists so a differential run can start BEFORE (the old
`domain_output` files) and AFTER (`gen/route_learner.json`) from the same learned names.

    python3 gen_to_store.py <gen dir> <out json> [--now SECONDS]
"""
import argparse
import json
import pathlib
import re
import time

CATEGORIES = {
    "sv4": "my_sv4list",
    "realip": "my_realiplist",
    "fakeip": "my_fakeiplist",
    "nov6": "my_nov6list",
    "nov4": "my_nov4list",
}
NAME = re.compile(r"[a-z0-9_*-]+(\.[a-z0-9_-]+)+")


def clean(name):
    name = name.strip().lower().rstrip(".")
    return name if len(name) <= 253 and NAME.fullmatch(name) else None


def names_of(path):
    """Names of a `*list.txt` (`count [date] name[|AD|CD]`) and a `*rule.txt` (`full:name`) file."""
    found = set()
    if not path.exists():
        return found
    for line in path.read_text(errors="replace").splitlines():
        fields = line.split()
        if not fields:
            continue
        if fields[0].startswith("full:"):
            name = clean(fields[0][5:])
        elif len(fields) >= 2:
            name = clean(fields[-1].split("|")[0])
        else:
            name = None
        if name:
            found.add(name)
    return found


def convert(gen, out, now=None):
    now = int(now if now is not None else time.time())
    categories = {}
    for stem, tag in CATEGORIES.items():
        names = names_of(gen / f"{stem}list.txt") | names_of(gen / f"{stem}rule.txt")
        categories[tag] = [[name, now, now, 0, "imported_for_test"] for name in sorted(names)]
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps({"format": 1, "saved_at": now, "categories": categories}))
    return {tag: len(rows) for tag, rows in categories.items()}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("gen", type=pathlib.Path)
    parser.add_argument("out", type=pathlib.Path)
    parser.add_argument("--now", type=int)
    arguments = parser.parse_args()
    print(json.dumps(convert(arguments.gen, arguments.out, arguments.now)))
