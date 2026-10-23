"""VM-only differential: BEFORE (old binary + old `config_rust`) against AFTER (new binary + new `config_rust`).

    python3 run.py --inputs I --work W --before B --after A --migrate M [--names N] [--groups ABCDEF]

`I` holds `config_all` (only for e2e-1's preparation), `config_rust` (the OLD tree), `config_rust_new` and
`snapshot/{rule,gen,srs,adguard,webinfo}`. The snapshot and every output stay outside the repository; the
summary written to `W/summary.json` has numbers and corpus indexes only. Each group runs BEFORE then AFTER, serially,
each in its own loopback-only network namespace with its own mocks. Groups: A cold start, B learned state, C restart,
D expiry, E SERVFAIL, F hot applies under load, G re-query flows and negative learning. See `README.md`.
"""
import argparse
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
E2E = HERE.parent / "e2e-1"
sys.path.insert(0, str(E2E))
sys.path.insert(0, str(HERE))
import classify  # noqa: E402
import gen_to_store  # noqa: E402
from compare import upstream_group  # noqa: E402
from prepare import prepare  # noqa: E402

TTL_TEST = 60
TAGS = ("my_sv4list", "my_realiplist", "my_fakeiplist", "my_nov6list", "my_nov4list")
# `save` writes the old configuration's rule files and `publish` posts them into its rule providers (the mapper
# then holds the learned names as rules, as it does a few hours into a normal run); the new learner only writes its file.
SAVE_ALL = [{"op": "get", "path": f"/plugins/{tag}/save"} for tag in TAGS] + [{"op": "publish"}, {"op": "sleep", "seconds": 4}]


def command(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def service_identity():
    return subprocess.check_output(["systemctl", "show", "mosdns", "-p", "MainPID", "-p", "ActiveState"])


class Rig:
    """One role's namespace, mock and host."""

    def __init__(self, work, name, root, binary, corpus, audit_ports, owned, namespaces):
        self.work, self.name, self.root, self.binary = work, name, root, binary
        self.corpus, self.audit_ports = corpus, audit_ports
        self.owned, self.namespaces = owned, namespaces
        self.ns = f"rl-{os.getpid()}-{name}"
        self.host = self.mock = None
        self.logs = []
        self.steps = 0

    def spawn(self, argv, filename, env=None):
        stream = (self.root / filename).open("a")
        self.logs.append(stream)
        process = subprocess.Popen(["ip", "netns", "exec", self.ns, *map(str, argv)], stdout=stream,
                                   stderr=subprocess.STDOUT, env=env, start_new_session=True)
        self.owned.append(process)
        return process

    def open(self):
        (self.root / "tmp").mkdir(exist_ok=True)
        quiet = {"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}
        root = self.root
        command(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", root / "ca-key.pem", "-out", root / "ca.pem", "-days", "2", "-subj", "/CN=RL-local-CA", "-addext", "basicConstraints=critical,CA:TRUE"], **quiet)
        command(["openssl", "req", "-newkey", "rsa:2048", "-nodes", "-keyout", root / "key.pem", "-out", root / "leaf.csr", "-subj", "/CN=localhost"], **quiet)
        (root / "leaf.ext").write_text("subjectAltName=DNS:localhost,IP:127.0.0.1\nbasicConstraints=critical,CA:FALSE\nextendedKeyUsage=serverAuth\n")
        command(["openssl", "x509", "-req", "-in", root / "leaf.csr", "-CA", root / "ca.pem", "-CAkey", root / "ca-key.pem", "-CAcreateserial", "-out", root / "cert.pem", "-days", "2", "-extfile", root / "leaf.ext"], **quiet)
        command(["ip", "netns", "add", self.ns])
        self.namespaces.append(self.ns)
        command(["ip", "-n", self.ns, "link", "set", "lo", "up"])
        self.mock = self.spawn(["python3", E2E / "mock.py", root], "mock.log")
        deadline = time.monotonic() + 10
        while not (root / "mock-ready").exists():
            if self.mock.poll() is not None or time.monotonic() > deadline:
                raise RuntimeError("mock failed readiness")
            time.sleep(.1)

    def start_host(self):
        env = dict(os.environ, SSL_CERT_FILE=str(self.root / "ca.pem"), SSL_CERT_DIR=str(self.root / "empty-certs"), PYTHONDONTWRITEBYTECODE="1")
        for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
            env.pop(key, None)
        self.host = self.spawn(["unshare", "--mount", "--propagation", "private", "sh", "-c", 'mount --bind "$1" /tmp; shift; exec "$@"', "rl", self.root / "tmp", self.binary, "start", "-d", self.root, "-c", "config_custom.yaml"], "host.log", env)

    def run_plan(self, plan):
        self.steps += 1
        plan_file = self.root / f"plan-{self.steps}.json"
        plan_file.write_text(json.dumps(plan))
        api_port = json.loads((self.root / "webinfo/webui_port_settings.json").read_text())["port"]
        command(["ip", "netns", "exec", self.ns, "python3", HERE / "agent.py", self.root, self.work / "corpus.json",
                 self.work / "audit-ports.json", plan_file, api_port, self.host.pid])
        if self.host.poll() is not None:
            raise RuntimeError(self.name + " host exited during the plan")

    def stop_host(self):
        if self.host is not None and self.host.poll() is None:
            os.killpg(self.host.pid, signal.SIGTERM)
            self.host.wait(timeout=60)
        self.host = None

    def close(self):
        self.stop_host()
        if self.mock is not None and self.mock.poll() is None:
            os.killpg(self.mock.pid, signal.SIGTERM)
            self.mock.wait(timeout=30)
        for stream in self.logs:
            stream.close()


def rows_of(path):
    return [json.loads(line) for line in path.read_text().splitlines()]


def sets_of(path):
    return json.loads(path.read_text())


def normalize_old_state(gen):
    """Makes the former lists consistent, as `domain_output` writes them: every name of a category's statistics
    file is in its rule file and the other way round (the snapshot has a few names in one only, and the
    preparation adds probe names to the rule files only)."""
    for stem in gen_to_store.CATEGORIES:
        names = sorted(gen_to_store.names_of(gen / f"{stem}list.txt") | gen_to_store.names_of(gen / f"{stem}rule.txt"))
        (gen / f"{stem}list.txt").write_text("".join(f"0000000001 2026-01-01 {name}\n" for name in names))
        (gen / f"{stem}rule.txt").write_text("".join(f"full:{name}\n" for name in names))


def make_roots(work, group, prepared, new_tree, before_state, after_state, ttl=None):
    """Copies the prepared (old-config) root twice; AFTER gets the new tree and its learned state."""
    base = work / group
    before, after = base / "before", base / "after"
    shutil.copytree(prepared, before)
    shutil.copytree(prepared, after)
    for root in (before, after):
        (root / "cache").mkdir(exist_ok=True)
        for dump in (root / "cache").glob("*.dump"):
            dump.unlink()
    if before_state == "as-is":
        normalize_old_state(before / "gen")
    if before_state == "cold":
        for path in (before / "gen").glob("*"):
            if path.name != "top_domains.txt":
                path.write_text("")
    # AFTER: the new configuration files over the old ones; the old learned files are not used.
    for path in list((after / "sub_config").glob("*.yaml")):
        path.unlink()
    shutil.copy(new_tree / "config_custom.yaml", after / "config_custom.yaml")
    for path in (new_tree / "sub_config").glob("*.yaml"):
        shutil.copy(path, after / "sub_config" / path.name)
    if ttl is not None:
        learner = after / "sub_config/route_learner.yaml"
        import re
        learner.write_text(re.sub(r"ttl: \d+", f"ttl: {ttl}", learner.read_text()))
    counts = None
    if after_state == "converted":
        counts = gen_to_store.convert(before / "gen", after / "gen/route_learner.json")
    for path in (after / "gen").glob("*"):
        if path.name not in ("top_domains.txt", "route_learner.json"):
            path.unlink()
    if after_state == "cold":
        (after / "gen/route_learner.json").unlink(missing_ok=True)
    return before, after, counts


def run_pair(work, group, roots, plans, binaries, corpus, audit_ports, owned, namespaces, restart_plans=None):
    """Runs one plan per role (serially); `restart_plans` runs after a graceful stop and a start without cache dumps."""
    rigs = {}
    try:
        for role, root in zip(("before", "after"), roots):
            rig = Rig(work, f"{group}{role[0]}", root, binaries[role], corpus, audit_ports, owned, namespaces)
            rigs[role] = rig
            rig.open()
            rig.start_host()
            rig.run_plan(plans[role])
            if restart_plans:
                rig.stop_host()
                for dump in (root / "cache").glob("*.dump"):
                    dump.unlink()
                rig.start_host()
                rig.run_plan(restart_plans[role])
            rig.close()
    finally:
        for rig in rigs.values():
            rig.close()
    return rigs


def upstream_mapper(work):
    mapping = json.loads((work / "mapping.json").read_text())["upstream_aliases"]["native"]
    return lambda audit: upstream_group(audit, {k: v for k, v in mapping.items()})


def verdict(ok):
    return "PASS" if ok else "FAIL"


def compare_tag(before, after, tag, group_of, relaxed=False):
    failures, sources, count = classify.compare_rows(
        rows_of(before / f"{tag}.jsonl"), rows_of(after / f"{tag}.jsonl"), group_of,
        classify.LEARNED_SOURCES if relaxed else frozenset())
    return {"rows": count, "failures": failures, "matched_rule_source": sources}


def summarize_failures(result):
    return {"rows": result["rows"], "different_rows": len(result["failures"]),
            "fields": sorted({f for row in result["failures"] for f in row["fields"]}),
            "first_indexes": [(r["pass"], r["index"]) for r in result["failures"][:10]],
            "matched_rule_source": result["matched_rule_source"]}


def group_ab(work, prepared, new_tree, args, ctx, state):
    """A (cold) + C (restart), or B (learned state)."""
    name = state
    cold = name == "A"
    before, after, counts = make_roots(work, name, prepared, new_tree, "cold" if cold else "as-is",
                                       "cold" if cold else "converted")
    if cold:
        plan = [{"op": "ready"}, {"op": "replay", "tag": "p1", "pass_id": 1}, {"op": "dump", "tag": "d1"}, *SAVE_ALL,
                {"op": "replay", "tag": "p2", "pass_id": 2}, {"op": "dump", "tag": "d2"}, {"op": "rss", "tag": "rss"}]
        restart = [{"op": "ready"}, {"op": "dump", "tag": "r0"}, {"op": "replay", "tag": "r1", "pass_id": 3}, {"op": "dump", "tag": "r2"}]
    else:
        plan = [{"op": "ready"}, {"op": "dump", "tag": "d0"}, {"op": "replay", "tag": "p1", "pass_id": 1},
                {"op": "dump", "tag": "d1"}, *SAVE_ALL, {"op": "replay", "tag": "p2", "pass_id": 2}, {"op": "dump", "tag": "d2"},
                {"op": "rss", "tag": "rss"}]
        restart = None
    run_pair(work, name, (before, after), {"before": plan, "after": plan}, ctx["binaries"], ctx["corpus"],
             ctx["audit_ports"], ctx["owned"], ctx["namespaces"], {"before": restart, "after": restart} if restart else None)
    group_of = upstream_mapper(work)
    out = {"imported": counts}
    ok = True
    # Pass 1 learns while it runs (BEFORE's new names are in its in-memory overlay only: relaxed `matched_rule_source`);
    # pass 2 runs after `save` published them, the state a normal run is in a few hours in: strict.
    for tag in ("p1", "p2"):
        result = compare_tag(before, after, tag, group_of, tag == "p1")
        out[tag] = summarize_failures(result)
        ok &= not result["failures"]
    steps = ("d0", "d1", "d2") if not cold else ("d1", "d2")
    for tag in steps:
        diff = classify.set_differences(sets_of(before / f"{tag}.json"), sets_of(after / f"{tag}.json"))
        out["sets_" + tag] = {t: {k: len(v) for k, v in e.items()} for t, e in diff.items()}
        ok &= not diff
    out["learned"] = {t: len(v) for t, v in sets_of(after / "d2.json").items()}
    out["rss_kib"] = {"before": sets_of(before / "rss.rss.json")["rss_kib"], "after": sets_of(after / "rss.rss.json")["rss_kib"]}
    verdicts = {name: verdict(ok)}
    if cold:
        result = compare_tag(before, after, "r1", group_of)
        out["r1"] = summarize_failures(result)
        persisted = [classify.set_differences(sets_of(root / "d2.json"), sets_of(root / "r0.json")) for root in (before, after)]
        out["restart_state_kept"] = {"before": not persisted[0], "after": not persisted[1]}
        same = not classify.set_differences(sets_of(before / "r2.json"), sets_of(after / "r2.json"))
        verdicts["C"] = verdict(not result["failures"] and not any(persisted) and same)
    return out, verdicts


def group_d(work, prepared, new_tree, args, ctx):
    before, after, _ = make_roots(work, "D", prepared, new_tree, "cold", "cold", ttl=TTL_TEST)
    plans = {"before": [{"op": "ready"}, {"op": "replay", "tag": "p1", "pass_id": 1}, {"op": "dump", "tag": "d1"}],
             "after": [{"op": "ready"}, {"op": "replay", "tag": "p1", "pass_id": 1}, {"op": "dump", "tag": "d1"},
                       {"op": "sleep", "seconds": TTL_TEST + 5}, {"op": "flush_caches"}, {"op": "dump", "tag": "d2"},
                       {"op": "replay", "tag": "p2", "pass_id": 2}]}
    run_pair(work, "D", (before, after), plans, ctx["binaries"], ctx["corpus"], ctx["audit_ports"], ctx["owned"], ctx["namespaces"])
    group_of = upstream_mapper(work)
    out = {}
    learn = compare_tag(before, after, "p1", group_of, True)
    out["learning_pass"] = summarize_failures(learn)
    # An expired learned state is a never-learned state: AFTER's second pass (expired, caches flushed)
    # against BEFORE's first pass (cold), compared as the same (pass, index).
    expired_rows = rows_of(after / "p2.jsonl")
    for row in expired_rows:
        row["pass"] = 1
    fixed_after = classify.compare_rows(rows_of(before / "p1.jsonl"), expired_rows, group_of, classify.LEARNED_SOURCES)
    out["expired_vs_cold"] = summarize_failures({"rows": fixed_after[2], "failures": fixed_after[0], "matched_rule_source": fixed_after[1]})
    empty = all(not names for names in sets_of(after / "d2.json").values())
    out["learned_before_expiry"] = {t: len(v) for t, v in sets_of(after / "d1.json").items()}
    out["learned_after_expiry_empty"] = empty
    diff = classify.set_differences(sets_of(before / "d1.json"), sets_of(after / "d1.json"))
    out["sets_d1"] = {t: {k: len(v) for k, v in e.items()} for t, e in diff.items()}
    ok = not learn["failures"] and not fixed_after[0] and empty and not diff
    return out, {"D": verdict(ok)}


def group_e(work, prepared, new_tree, args, ctx):
    before, after, _ = make_roots(work, "E", prepared, new_tree, "cold", "cold")
    plan = [{"op": "ready"}, {"op": "faults", "value": {"groups": {"domestic": {"mode": "servfail"}}}},
            {"op": "replay", "tag": "p1", "pass_id": 1}, {"op": "dump", "tag": "d1"}, *SAVE_ALL,
            {"op": "replay", "tag": "p2", "pass_id": 2}, {"op": "dump", "tag": "d2"}, {"op": "faults", "value": None}]
    run_pair(work, "E", (before, after), {"before": plan, "after": plan}, ctx["binaries"], ctx["corpus"], ctx["audit_ports"], ctx["owned"], ctx["namespaces"])
    group_of = upstream_mapper(work)
    out = {"servfail_group": "domestic"}
    ok = True
    for tag in ("d1", "d2"):
        diff = classify.set_differences(sets_of(before / f"{tag}.json"), sets_of(after / f"{tag}.json"))
        allowed, unexpected = classify.servfail_allowance(diff)
        out["sets_" + tag] = {"servfail_names_not_learned_by_after": len(allowed),
                              "unexpected_sets": {t: {k: len(v) for k, v in e.items()} for t, e in unexpected.items()}}
        ok &= not unexpected
    diff = classify.set_differences(sets_of(before / "d2.json"), sets_of(after / "d2.json"))
    allowed, _ = classify.servfail_allowance(diff)
    for tag in ("p1", "p2"):
        result = compare_tag(before, after, tag, group_of, tag == "p1")
        explained, unexplained = classify.servfail_split(result["failures"], allowed)
        out[tag] = {"rows": result["rows"], "explained_by_servfail_learning": len(explained),
                    "unexplained": len(unexplained), "unexplained_fields": sorted({f for r in unexplained for f in r["fields"]}),
                    "first_unexplained": [(r["pass"], r["index"]) for r in unexplained[:10]]}
        ok &= not unexplained
    return out, {"E": verdict(ok)}


def group_g(work, prepared, new_tree, args, ctx):
    """Requery flows and negative learning: the `empty` mock mode answers NOERROR without records."""
    before, after, _ = make_roots(work, "G", prepared, new_tree, "cold", "cold")
    empty = lambda *groups: {"groups": {g: {"mode": "empty"} for g in groups}}  # noqa: E731
    requery = 3066  # `sequence_requery`: the flows only the re-query task reaches
    plan = [{"op": "ready"},
            # The domestic resolver has nothing, the foreign one answers: special names, remembered proxy.
            {"op": "faults", "value": empty("domestic")},
            {"op": "replay", "tag": "g1", "pass_id": 1, "port": requery}, {"op": "dump", "tag": "d1"},
            # Nobody has an address: remembered no-V4 / no-V6 through the re-query flows...
            {"op": "faults", "value": empty("domestic", "foreign", "foreignecs")}, {"op": "flush_caches"},
            {"op": "replay", "tag": "g2", "pass_id": 2, "port": requery}, {"op": "dump", "tag": "d2"},
            # ...and the learned negatives answer at once on the normal port.
            *SAVE_ALL, {"op": "replay", "tag": "g3", "pass_id": 3}, {"op": "dump", "tag": "d3"},
            {"op": "faults", "value": None}]
    run_pair(work, "G", (before, after), {"before": plan, "after": plan}, ctx["binaries"], ctx["corpus"], ctx["audit_ports"], ctx["owned"], ctx["namespaces"])
    group_of = upstream_mapper(work)
    out, ok = {}, True
    for tag in ("g1", "g2", "g3"):
        result = compare_tag(before, after, tag, group_of, False)
        out[tag] = summarize_failures(result)
        ok &= not result["failures"]
    for tag in ("d1", "d2", "d3"):
        diff = classify.set_differences(sets_of(before / f"{tag}.json"), sets_of(after / f"{tag}.json"))
        out["sets_" + tag] = {t: {k: len(v) for k, v in e.items()} for t, e in diff.items()}
        ok &= not diff
    out["learned"] = {t: len(v) for t, v in sets_of(after / "d3.json").items()}
    return out, {"G": verdict(ok)}


def group_f(work, prepared, new_tree, args, ctx):
    _, after, _ = make_roots(work, "F", prepared, new_tree, "cold", "cold")
    plan = [{"op": "ready"}, {"op": "replay", "tag": "p1", "pass_id": 1}, {"op": "dump", "tag": "d1"}, {"op": "rss", "tag": "rss1"},
            {"op": "applies", "tag": "p2", "count": 6, "pass_id": 2}, {"op": "dump", "tag": "d2"},
            {"op": "sleep", "seconds": 30}, {"op": "rss", "tag": "rss2"}]
    rig_root = after
    run_pair_single(work, "F", rig_root, plan, ctx)
    # The same load without the applies, for the memory comparison.
    _, control, _ = make_roots(work, "Fc", prepared, new_tree, "cold", "cold")
    control_plan = [{"op": "ready"}, {"op": "replay", "tag": "p1", "pass_id": 1}, {"op": "rss", "tag": "rss1"},
                    {"op": "replay", "tag": "p2", "pass_id": 2}, {"op": "sleep", "seconds": 30}, {"op": "rss", "tag": "rss2"}]
    run_pair_single(work, "Fc", control, control_plan, ctx)
    # BEFORE (previous program, old configuration) under the same applies, for context.
    old, _, _ = make_roots(work, "Fb", prepared, new_tree, "cold", "cold")
    run_pair_single(work, "Fb", old, plan, ctx, "before")
    rows = rows_of(after / "p2.jsonl")
    errors = sum(1 for r in rows if "error" in r["dns"])
    d1, d2 = sets_of(after / "d1.json"), sets_of(after / "d2.json")
    lost = {t: len(set(d1[t]) - set(d2[t])) for t in d1}
    applies = sets_of(after / "p2.applies.json")["failures"]
    def spread(path):
        values = sorted(r["latency_ms"] for r in rows_of(path))
        return {"p50_ms": round(values[len(values) // 2], 1), "p99_ms": round(values[int(len(values) * .99)], 1), "max_ms": round(values[-1], 1)}

    latency = {"during_applies": spread(after / "p2.jsonl"), "control_same_pass": spread(control / "p2.jsonl"),
               "before_program_during_applies": spread(old / "p2.jsonl")}
    out = {"latency": latency, "rows_under_applies": len(rows), "dns_errors": errors, "apply_failures": len(applies),
           "learned_lost_across_applies": lost, "learned_before": {t: len(v) for t, v in d1.items()},
           "rss_kib": {"before_applies": sets_of(after / "rss1.rss.json")["rss_kib"], "after_applies": sets_of(after / "rss2.rss.json")["rss_kib"],
                       "control_second_pass": sets_of(control / "rss2.rss.json")["rss_kib"]}}
    out["before_program_rss_kib"] = {"before_applies": sets_of(old / "rss1.rss.json")["rss_kib"], "after_applies": sets_of(old / "rss2.rss.json")["rss_kib"]}
    out["rss_ratio_to_control"] = round(out["rss_kib"]["after_applies"] / out["rss_kib"]["control_second_pass"], 3)
    ok = errors == 0 and not applies and not any(lost.values()) and out["rss_ratio_to_control"] <= 1.2 and latency["during_applies"]["max_ms"] < 2000
    return out, {"F": verdict(ok)}


def run_pair_single(work, group, root, plan, ctx, role="after"):
    rig = Rig(work, f"{group}{role[0]}", root, ctx["binaries"][role], ctx["corpus"], ctx["audit_ports"], ctx["owned"], ctx["namespaces"])
    try:
        rig.open()
        rig.start_host()
        rig.run_plan(plan)
    finally:
        rig.close()


def main(args):
    if os.geteuid() != 0:
        raise RuntimeError("run as root on the test VM")
    work = args.work
    work.mkdir(mode=0o700)
    before_service = service_identity()
    ctx = {"owned": [], "namespaces": [], "binaries": {"before": args.before, "after": args.after}}
    summary = {"verdicts": {}, "groups": {}}
    try:
        prepare(work, args.inputs, args.migrate, args.names)
        ctx["corpus"] = json.loads((work / "corpus.json").read_text())
        ctx["audit_ports"] = json.loads((work / "audit-ports.json").read_text())
        prepared = work / "native"
        new_tree = args.inputs / "config_rust_new"
        # `rtdns check` on both trees: zero gaps.
        for role, binary, tree in (("before", args.before, prepared), ("after", args.after, None)):
            if tree is None:
                continue
            command([binary, "check", "-d", tree, "-c", "config_custom.yaml"], stdout=(work / f"check-{role}.txt").open("w"), stderr=subprocess.STDOUT)
        for group in args.groups:
            started = time.monotonic()
            if group in ("A", "B"):
                out, verdicts = group_ab(work, prepared, new_tree, args, ctx, group)
                summary["groups"][group] = out
            elif group == "D":
                out, verdicts = group_d(work, prepared, new_tree, args, ctx)
                summary["groups"]["D"] = out
            elif group == "E":
                out, verdicts = group_e(work, prepared, new_tree, args, ctx)
                summary["groups"]["E"] = out
            elif group == "F":
                out, verdicts = group_f(work, prepared, new_tree, args, ctx)
                summary["groups"]["F"] = out
            elif group == "G":
                out, verdicts = group_g(work, prepared, new_tree, args, ctx)
                summary["groups"]["G"] = out
            elif group == "C":
                continue
            else:
                raise ValueError(group)
            summary["verdicts"].update(verdicts)
            summary["groups"][group]["seconds"] = round(time.monotonic() - started)
            (work / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    finally:
        for process in reversed(ctx["owned"]):
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
        for ns in reversed(ctx["namespaces"]):
            if subprocess.check_output(["ip", "netns", "pids", ns]).strip():
                raise RuntimeError("owned namespace still has processes")
            command(["ip", "netns", "delete", ns])
        after_service = service_identity()
        (work / "cleanup.json").write_text(json.dumps({"processes_exited": all(p.poll() is not None for p in ctx["owned"]),
                                                      "namespaces_deleted": ctx["namespaces"],
                                                      "original_service_unchanged": before_service == after_service}, indent=2))
        if before_service != after_service:
            raise RuntimeError("original service identity changed")
    print(json.dumps(summary["verdicts"], indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    for flag in ("work", "inputs", "before", "after", "migrate"):
        parser.add_argument("--" + flag, required=True, type=lambda s: pathlib.Path(s).resolve())
    parser.add_argument("--names", type=int, default=1500)
    parser.add_argument("--groups", default="ABDEFG")
    main(parser.parse_args())
