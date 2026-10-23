"""VM-only differential of M2-1: BEFORE (the schema-2 program + old `config_rust`, switches in `rule/switchN.txt`)
against AFTER (the new program + schema-3 `config_rust`, named flags), for many flag combinations.

    python3 run.py --inputs I --work W --before B --after A --migrate M [--names N] [--groups G0,G1,...]

`I` is laid out as for `tools/route-learner` (`config_all`, `config_rust` = OLD tree, `config_rust_new`,
`snapshot/{rule,gen,srs,adguard,webinfo}`). Both sides start from the same learned state (the snapshot's lists
converted to `gen/route_learner.json`; both programs use `route_learner`) and the same mocks and corpus. Every
combination replays the corpus twice on the normal port (caches cold, then warm) and once on the re-query port,
strictly: any difference fails. See `README.md`.
"""
import argparse
import json
import os
import pathlib
import shutil
import sys
import time

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "route-learner"))
sys.path.insert(0, str(HERE.parent / "e2e-1"))
import importlib.util  # noqa: E402

import classify  # noqa: E402
import flags  # noqa: E402  (this directory)
import gen_to_store  # noqa: E402
from prepare import prepare  # noqa: E402

# tools/route-learner/run.py (a module named `run` like this script): its rig, replay and comparison helpers.
_spec = importlib.util.spec_from_file_location("route_learner_run", HERE.parent / "route-learner/run.py")
rl = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(rl)

REQUERY_PORT = 3066


def extend_corpus(work):
    """Adds SOA/PTR/HTTPS questions (the `block_qtypes` rules) for a few production names and every probe."""
    corpus = json.loads((work / "corpus.json").read_text())
    extra = []
    tops = [row for row in corpus if row["class"] == "top" and row["type"] == 1][:20]
    probes = [row for row in corpus if row["class"] not in ("top", "listener") and row["type"] == 1]
    for row in tops + probes:
        for qtype in (6, 12, 65):
            extra.append(dict(row, type=qtype))
    corpus.extend(extra)
    (work / "corpus.json").write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + "\n")
    return len(corpus), len(extra)


def make_roots(work, name, prepared, new_tree, combination):
    """BEFORE: prepared old root, the combination's switch files. AFTER: the new tree; the values either in the
    switch files (migrated at start) or written into `settings.json` `flags` directly."""
    base = work / name
    before, after = base / "before", base / "after"
    shutil.copytree(prepared, before)
    shutil.copytree(prepared, after)
    for root in (before, after):
        for dump in (root / "cache").glob("*.dump"):
            dump.unlink()
        for number in flags.ALL_SWITCHES:
            (root / f"rule/switch{number}.txt").unlink(missing_ok=True)
    letters = flags.switch_letters(combination["switches"])
    for number, letter in letters.items():
        (before / f"rule/switch{number}.txt").write_text(letter)
    letters = flags.switch_letters(combination.get("after_switches", combination["switches"]))
    for path in list((after / "sub_config").glob("*.yaml")):
        if path.name != "special_groups.yaml":
            path.unlink()
    shutil.copy(new_tree / "config_custom.yaml", after / "config_custom.yaml")
    for path in (new_tree / "sub_config").glob("*.yaml"):
        if path.name != "special_groups.yaml":
            shutil.copy(path, after / "sub_config" / path.name)
    settings_path = after / "webinfo/settings.json"
    settings = json.loads(settings_path.read_text())
    if combination["mode"] == "migrate":
        for number, letter in letters.items():
            (after / f"rule/switch{number}.txt").write_text(letter)
    else:
        settings["flags"] = flags.differences(letters)
    settings_path.write_text(json.dumps(settings, ensure_ascii=False, indent=2) + "\n")
    return before, after


def compare(before, after, tag, group_of):
    failures, sources, count = classify.compare_rows(rl.rows_of(before / f"{tag}.jsonl"),
                                                     rl.rows_of(after / f"{tag}.jsonl"), group_of)
    return failures, {"rows": count, "different_rows": len(failures),
                      "fields": sorted({f for row in failures for f in row["fields"]}),
                      "first_indexes": [(r["pass"], r["index"]) for r in failures[:10]],
                      "matched_rule_source": sources}


def run_combination(work, prepared, new_tree, ctx, name, combination):
    before, after = make_roots(work, name, prepared, new_tree, combination)
    passes = [{"op": "ready"}, {"op": "replay", "tag": "p1", "pass_id": 1}, {"op": "replay", "tag": "p2", "pass_id": 2},
              {"op": "replay", "tag": "r", "pass_id": 3, "port": REQUERY_PORT}, {"op": "dump", "tag": "d"}]
    rl.run_pair(work, name, (before, after), {"before": passes, "after": passes}, ctx["binaries"], ctx["corpus"],
                ctx["audit_ports"], ctx["owned"], ctx["namespaces"])
    return finish(work, before, after, ("p1", "p2", "r"), combination)


def finish(work, before, after, tags, combination):
    group_of = rl.upstream_mapper(work)
    out, ok = {"mode": combination["mode"]}, True
    for tag in tags:
        failures, summary = compare(before, after, tag, group_of)
        out[tag] = summary
        ok &= not failures
    diff = classify.set_differences(rl.sets_of(before / "d.json"), rl.sets_of(after / "d.json"))
    out["learned_sets_differ"] = {t: {k: len(v) for k, v in e.items()} for t, e in diff.items()}
    ok &= not diff
    expected = flags.differences(flags.switch_letters(combination.get("after_switches", combination["switches"])))
    written = json.loads((after / "webinfo/settings.json").read_text()).get("flags")
    out["flags_file_as_expected"] = written == expected
    ok &= written == expected
    return out, ok


def run_hot(work, prepared, new_tree, ctx, name, combination):
    """Both start at the package defaults; after one pass both switch to `combination` through their own API
    (BEFORE `/plugins/switchN/post`, AFTER `/api/v1/flags/{key}`), make one identical settings apply, then pass 2
    and the re-query pass."""
    defaults = {"mode": "migrate", "switches": {}}
    before, after = make_roots(work, name, prepared, new_tree, defaults)
    letters = flags.switch_letters(combination["switches"])
    old_steps = [{"op": "post", "path": f"/plugins/switch{n}/post", "body": {"value": v}}
                 for n, v in sorted(combination["switches"].items())]
    new_steps = [{"op": "post", "path": f"/api/v1/flags/{key}", "body": {"value": value}}
                 for key, value in sorted(flags.differences(letters).items())]
    # The new flag change is a whole-generation apply, the old switch POST is not. Names learned in pass 1 carry
    # their learned label in `domain_set` only from the next generation on (in both programs), so both sides make
    # the same extra settings apply (the proxy override, as group F of tools/route-learner) before pass 2.
    regenerate = {"op": "post", "path": "/api/v1/overrides",
                  "body": {"socks5": "127.0.0.1:17892", "ecs": "", "replacements": []}}
    tail = [regenerate, {"op": "sleep", "seconds": 2}, {"op": "replay", "tag": "p2", "pass_id": 2},
            {"op": "replay", "tag": "r", "pass_id": 3, "port": REQUERY_PORT}, {"op": "dump", "tag": "d"}]
    head = [{"op": "ready"}, {"op": "replay", "tag": "p1", "pass_id": 1}]
    rl.run_pair(work, name, (before, after), {"before": head + old_steps + tail, "after": head + new_steps + tail},
                ctx["binaries"], ctx["corpus"], ctx["audit_ports"], ctx["owned"], ctx["namespaces"])
    out, ok = finish(work, before, after, ("p1", "p2", "r"), combination)
    # BEFORE's switch files now hold the combination as well.
    files = {n: (before / f"rule/switch{n}.txt").read_text().strip() for n in combination["switches"]}
    out["before_switch_files_written"] = files == {n: letters[n] for n in combination["switches"]}
    return out, ok and out["before_switch_files_written"]


def main(args):
    if os.geteuid() != 0:
        raise RuntimeError("run as root on the test VM")
    work = args.work
    work.mkdir(mode=0o700)
    before_service = rl.service_identity()
    ctx = {"owned": [], "namespaces": [], "binaries": {"before": args.before, "after": args.after}}
    summary = {"verdicts": {}, "groups": {}}
    try:
        prepare(work, args.inputs, args.migrate, args.names)
        rows, extra = extend_corpus(work)
        summary["corpus"] = {"queries_per_pass": rows, "qtype_6_12_65_rows": extra}
        ctx["corpus"] = json.loads((work / "corpus.json").read_text())
        ctx["audit_ports"] = json.loads((work / "audit-ports.json").read_text())
        prepared = work / "native"
        # The same learned state on both sides (both programs keep it in `gen/route_learner.json`).
        rl.normalize_old_state(prepared / "gen")
        summary["learned_state"] = gen_to_store.convert(prepared / "gen", prepared / "gen/route_learner.json")
        for path in (prepared / "gen").glob("*"):
            if path.name not in ("top_domains.txt", "route_learner.json"):
                path.unlink()
        new_tree = args.inputs / "config_rust_new"
        wanted = args.groups.split(",") if args.groups else list(flags.COMBINATIONS)
        for name in wanted:
            combination = flags.COMBINATIONS[name]
            started = time.monotonic()
            if combination["mode"] == "hot":
                out, ok = run_hot(work, prepared, new_tree, ctx, name, combination)
            else:
                out, ok = run_combination(work, prepared, new_tree, ctx, name, combination)
            out["seconds"] = round(time.monotonic() - started)
            out["flags"] = flags.differences(flags.switch_letters(combination["switches"]))
            summary["groups"][name] = out
            if "after_switches" in combination:
                # A negative control passes when the comparison does find the difference.
                ok = out["flags_file_as_expected"] and any(out[t]["different_rows"] for t in ("p1", "p2", "r"))
            summary["verdicts"][name] = rl.verdict(ok)
            (work / "summary.json").write_text(json.dumps(summary, indent=2, sort_keys=True) + "\n")
    finally:
        rl_cleanup(work, ctx, before_service)
    print(json.dumps(summary["verdicts"], indent=2))


def rl_cleanup(work, ctx, before_service):
    import signal
    import subprocess
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
        rl.command(["ip", "netns", "delete", ns])
    after_service = rl.service_identity()
    (work / "cleanup.json").write_text(json.dumps({"processes_exited": all(p.poll() is not None for p in ctx["owned"]),
                                                  "namespaces_deleted": ctx["namespaces"],
                                                  "original_service_unchanged": before_service == after_service}, indent=2))
    if before_service != after_service:
        raise RuntimeError("original service identity changed")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    for flag in ("work", "inputs", "before", "after", "migrate"):
        parser.add_argument("--" + flag, required=True, type=lambda s: pathlib.Path(s).resolve())
    parser.add_argument("--names", type=int, default=500)
    parser.add_argument("--groups", default="")
    main(parser.parse_args())
