#!/usr/bin/env python3
"""Dual-stack consistency probe: what does one tree do with an unlisted name whose A and
AAAA answers disagree about being domestic (e.g. a CN IPv4 but a non-CN IPv6)?

    python3 mixed.py --tree <config_rust> --binary <atomdns> --work <fresh dir> \
        [--combos compat,safe] [--delay-a 0.8] [--delay-aaaa 0] [--no-fake-v6]

Each name is asked in rounds: r1 the first contact (A then AAAA, AAAA then A, or both at
once as a happy-eyeballs client does), r2 right after, r3 after the 5 s retention of an
empty answer has run out (a lazy cache serves the stale copy and refreshes), r4 after that
refresh. After r1 it prints what the route learner remembered for the name.

In the FakeIP combos a row is flagged when the families ended on different sides in r4,
when the learned routes disagree with A, or when r1 gave a real AAAA although A has a
record (the A verdict should win there).
"""
import argparse
import json
import os
import sys
import threading
import time
import urllib.parse

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import run  # noqa: E402

# (domestic code, foreign code): A_AAAA per side
PAIRS = [
    ("cn_fo", "fo"),    # domestic: CN v4 + non-CN v6; foreign: plain foreign
    ("fo_cn", "fo"),    # domestic: foreign v4 + CN v6
    ("cn_cn", "fo"),    # control: both CN
    ("fo_fo", "fo"),    # control: both foreign
    ("cn_none", "fo"),  # CN v4, no v6 at domestic
    ("none_cn", "fo"),  # no v4, CN v6 (pure IPv6 name in compat mode)
    ("fo", "cn_fo"),    # foreign (safe mode asks only it): CN v4 + non-CN v6
    ("fo", "fo_cn"),    # foreign: non-CN v4 + CN v6
    ("fo", "none_cn"),  # foreign: no v4, CN v6 (pure IPv6 name in safe mode)
]
ORDERS = {"A,AAAA": (1, 28), "AAAA,A": (28, 1), "PAR": (28, 1)}
FAKE = {"28.0.0.7": "proxy", "2001:2::7": "proxy", "30.0.0.7": "direct", "2400::7": "direct"}
LEARNED = {"my_realiplist": "直连", "my_fakeiplist": "代理", "my_nov6list": "无V6"}


def ask(name, qtype):
    response = run.ask(run.DNS, name, qtype)
    return [a[1] for a in response["answers"]] or [response["rcode"] if response["rcode"] else "empty"]


def remembered(name):
    found = []
    for tag, label in LEARNED.items():
        body = run.http("GET", f"/plugins/{tag}/show?q={urllib.parse.quote(name)}").decode()
        if any(line.endswith(" " + name) for line in body.splitlines()):
            found.append(label)
    return found


def route(answers, qtype):
    """direct / proxy / none for one FakeIP-mode answer list."""
    value = answers[0]
    if value in FAKE:
        return FAKE[value]
    if value in ("empty", 0):
        return "none"
    if isinstance(value, str) and (":" in value or "." in value):
        return "direct"  # a real address: FakeIP mode only hands those out for direct names
    return "none"


def judge(combo, rows, memory):
    if "realip" in combo:
        return ""
    problems = []
    a4, aaaa4 = route(rows["r4"][1], 1), route(rows["r4"][28], 28)
    if a4 != "none" and aaaa4 != "none" and a4 != aaaa4:
        problems.append("SPLIT")
    expected = {"direct": ["直连"], "proxy": ["代理"], "none": []}[a4]
    if [m for m in memory if m != "无V6"] != expected:
        problems.append(f"MEMORY≠A({a4})")
    if route(rows["r1"][1], 1) != "none" and route(rows["r1"][28], 28) == "direct" and a4 == "proxy":
        problems.append("R1-REAL-AAAA")
    return " ".join(problems)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tree", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--work", required=True)
    parser.add_argument("--combos", default="compat,safe,compat+cnfake,safe+cnfake")
    parser.add_argument("--delay-a", type=float, default=0.0)
    parser.add_argument("--delay-aaaa", type=float, default=0.0)
    parser.add_argument("--no-fake-v6", action="store_true")
    parser.add_argument("--json")
    args = parser.parse_args()
    os.makedirs(args.work)
    mocks = run.Mocks(delays={1: args.delay_a, 28: args.delay_aaaa}, fake_v6=not args.no_fake_v6)
    dump = {}
    flagged = 0
    for combo in args.combos.split(","):
        directory = run.stage(args.tree, os.path.join(args.work, combo), run.FLAGS[combo])
        process = run.start(args.binary, directory)
        try:
            names = []
            for d, f in PAIRS:
                for order in ORDERS:
                    names.append((d, f, order, f"{d}.{f}.unl-x{len(names)}.unl.test"))
            rows = {name: {} for *_, name in names}

            def first(order, name):
                results = {}
                if order == "PAR":
                    threads = [threading.Thread(target=lambda q=q: results.__setitem__(q, ask(name, q)))
                               for q in ORDERS[order]]
                    for thread in threads:
                        thread.start()
                    for thread in threads:
                        thread.join()
                else:
                    for qtype in ORDERS[order]:
                        results[qtype] = ask(name, qtype)
                rows[name]["r1"] = results

            workers = [threading.Thread(target=first, args=(order, name)) for _, _, order, name in names]
            for worker in workers:
                worker.start()
            for worker in workers:
                worker.join()
            time.sleep(0.3)
            memory = {name: remembered(name) for *_, name in names}
            for label, pause in (("r2", 0), ("r3", 6), ("r4", 1)):
                time.sleep(pause)
                for *_, name in names:
                    rows[name][label] = {1: ask(name, 1), 28: ask(name, 28)}
            print(f"\n=== {combo}")
            for d, f, order, name in names:
                verdict = judge(combo, rows[name], memory[name])
                flagged += bool(verdict)
                rounds = "  ".join(f"{r}: A={rows[name][r][1]} AAAA={rows[name][r][28]}" for r in ("r1", "r2", "r3", "r4"))
                print(f"  domestic={d:8} foreign={f:8} {order:7} learned={memory[name]} {verdict}\n      {rounds}")
                dump[f"{combo}|{d}|{f}|{order}"] = {"rows": {r: {str(k): v for k, v in x.items()} for r, x in rows[name].items()},
                                                   "memory": memory[name], "verdict": verdict}
        finally:
            process.terminate()
            process.wait(timeout=20)
    print(f"\nflagged rows: {flagged}")
    if args.json:
        with open(args.json, "w") as handle:
            json.dump(dump, handle, indent=1, default=str)


if __name__ == "__main__":
    main()
