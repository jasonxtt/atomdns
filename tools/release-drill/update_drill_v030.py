#!/usr/bin/env python3
"""VM-only drill of the online update v0.2.2 -> v0.3.0 (configuration schema 2 -> 3, task 10-24-atomdns-release-v0.3.0).

Starts the REAL v0.2.2 release program as a systemd unit named `atomdns` (Restart=always, RestartSec=120)
in a loopback-only network namespace created and removed by this script (no public network, no clash
with other services of the machine). A fake GitHub API on 127.0.0.1 (ATOMDNS_UPDATE_API_URL) offers the
v0.3.0 candidate and the schema 3 configuration package.

    update_drill_v030.py --mode synthetic --work DIR --repo DIR --v022-dir DIR --release-dir DIR
                         --config-zip FILE --config-tar FILE
    update_drill_v030.py --mode prodcopy  --work DIR --repo DIR --v022-dir DIR --release-dir DIR
                         --config-zip FILE --prod-copy DIR

  --v022-dir     the v0.2.2 release assets (atomdns-linux-amd64.tar.gz + atomdns-update-manifest.json)
  --release-dir  the v0.3.0 output of scripts/build-rust-release.sh (with RUST_CONFIG_PACKAGE)
  --config-zip   the config_rust_up.zip that the metadata names (schema 3)
  --config-tar   synthetic: the schema 2 config_rust tree as a tar (file repository ccc64c9)
  --prod-copy    prodcopy: a private copy of a deployed root (never modified: the drill copies it again)

synthetic: user data made through the v0.2.2 program (proxy override, a special group with list and port,
learned names, non-default switch values, appearance files); scenarios 2 (config package sha256 refused),
3 (unhealthy candidate rolled back), 1 (success path) and 4 (no update, force).
prodcopy: the same update on a working copy of a deployed root: scenario 3 then 1, with only counts and
yes/no results printed (the copy's own values never reach the output).
Needs root, systemd, iproute2, jq and tar. Removes the unit, the namespace, the processes and (unless --keep)
the work directory when it ends, whatever the outcome.
"""
import argparse
import json
import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import importlib.util
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("update_drill", HERE / "update_drill.py")
u = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(u)
base = u.base

say, sha256, run, wait_for, dns_query = base.say, base.sha256, base.run, base.wait_for, base.dns_query
ASSET, MANIFEST, ANSWER = base.ASSET, base.MANIFEST, base.ANSWER
GROUP_ANSWER = "198.51.100.7"
GROUP_NAME = "drillgrp"
GROUP_PORT = 5301
GROUP_LIST = ["full:group.example.com", "domain:grp.example.com"]
LEARN = ["learn1.example.com", "learn2.example.com", "learn3.example.com"]
USER_RULE = "full:userrule.example.com\n"
CATEGORIES = ("my_nov6list", "my_nov4list", "my_realiplist", "my_fakeiplist", "my_sv4list")
FLAG_KEYS = ("core_mode", "dns_mode", "cn_fakeip", "prefer_v4", "ddns_foreign", "block_empty", "block_blocklist",
             "block_qtypes", "block_ads", "block_aaaa", "lazy_cache", "full_cache", "client_routing")
EXPECTED_SYNTHETIC_FLAGS = {"core_mode": "safe", "client_routing": "listed_direct", "ddns_foreign": True, "block_aaaa": True}


def start_upstream(port, answer):
    """base.start_upstream with a chosen A answer (the special group gets its own)."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("127.0.0.1", port))

    def serve():
        while True:
            try:
                data, peer = sock.recvfrom(2048)
            except OSError:
                return
            end = data.index(b"\0", 12) + 1
            qtype = struct.unpack(">H", data[end:end + 2])[0]
            question = data[12:end + 4]
            record = b""
            if qtype == 1:
                record = b"\xc0\x0c" + struct.pack(">HHIH", 1, 1, 60, 4) + socket.inet_aton(answer)
            header = data[:2] + struct.pack(">HHHHH", 0x8180, 1, 1 if record else 0, 0, 0)
            sock.sendto(header + question + record, peer)

    threading.Thread(target=serve, daemon=True).start()
    return sock


# ----- independent expectations ----------------------------------------------------------------

def flag_declarations(flags_yaml):
    """{key: (kind, default)} read straight from the YAML text of sub_config/flags.yaml."""
    out = {}
    for key, args in re.findall(r"- tag: (\w+)\n\s+type: feature_flag\n\s+args:(.*?)(?=\n  - tag:|\Z)", flags_yaml, re.S):
        kind = re.search(r"kind: (\w+)", args).group(1)
        default = re.search(r"default: (\S+)", args).group(1)
        out[key] = (kind, default == "true" if kind == "bool" else default)
    return out


def switch_values(root):
    """{n: 'A'|'B'|None}: the contract's reading of rule/switchN.txt (trimmed, exactly A or B)."""
    out = {}
    for n in range(1, 18):
        try:
            text = (Path(root) / f"rule/switch{n}.txt").read_bytes()[:1 << 20].decode().strip()
        except (OSError, UnicodeDecodeError):
            text = None
        out[n] = text if text in ("A", "B") else None
    return out


def expected_flags(root, declarations):
    """The migration table of native-feature-flags.md applied to the switch files: {key: value} for all 13."""
    sw = switch_values(root)
    value = {key: default for key, (_, default) in declarations.items()}
    two = {"core_mode": (3, "compat", "safe"), "dns_mode": (17, "fakeip", "realip")}
    for key, (n, a, b) in two.items():
        if sw[n]:
            value[key] = a if sw[n] == "A" else b
    if sw[9]:
        value["cn_fakeip"] = sw[9] == "B"
    for key, n in (("prefer_v4", 8), ("ddns_foreign", 16), ("block_qtypes", 5), ("block_ads", 7), ("block_aaaa", 6),
                   ("lazy_cache", 4), ("full_cache", 13), ("block_empty", 1), ("block_blocklist", 1)):
        if sw[n]:
            value[key] = sw[n] == "A"
    value["client_routing"] = {("A", "B"): "only_listed_proxy", ("B", "A"): "listed_direct"}.get((sw[2], sw[12]), "none")
    return value


def counts_of(learner_file):
    try:
        data = json.loads(Path(learner_file).read_text())
    except (OSError, ValueError):
        return {}
    return {name: len(rows) for name, rows in data.get("categories", {}).items()}


def names_of(learner_file):
    data = json.loads(Path(learner_file).read_text())
    return {name: sorted(row[0] for row in rows) for name, rows in data.get("categories", {}).items()}


def learner_ttls(route_learner_yaml):
    """{category: ttl seconds} from the package's sub_config/route_learner.yaml."""
    return {tag: int(ttl) for tag, ttl in re.findall(r"- tag: (my_\w+)[^\n]*\n(?:[^\n]*\n)*?\s+ttl: (\d+)", route_learner_yaml)}


def must_keep(learner_file, ttls, horizon=1800):
    """Names that stay live for at least `horizon` more seconds (an entry expires `ttl` after its last confirmation)."""
    data = json.loads(Path(learner_file).read_text())
    now = time.time()
    return {name: {row[0] for row in rows if row[2] + ttls[name] > now + horizon} for name, rows in data.get("categories", {}).items()}


def kept(must, before, after):
    """(every name of `must` is still in `after`, names `after` has in addition to `before`, names of `before` gone):
    the running program keeps learning from the drill's own probe queries, sweeps expired entries and saves its state at
    every hand-over and stop."""
    ok = all(rows <= set(after.get(c, [])) for c, rows in must.items())
    added = sum(len(set(rows) - set(before.get(c, []))) for c, rows in after.items())
    gone = sum(len(set(rows) - set(after.get(c, []))) for c, rows in before.items())
    return ok, added, gone


class Timeline:
    """Samples every 20 ms: the transaction state (update/txn-*/transaction.json) and the sha256 of watched files."""

    def __init__(self, root, files):
        self.root, self.files = Path(root), files
        self.samples, self.stop = [], threading.Event()
        self.thread = threading.Thread(target=self.loop, daemon=True)

    def loop(self):
        while not self.stop.is_set():
            state = None
            for txn in sorted(self.root.glob("update/txn-*/transaction.json")):
                try:
                    state = json.loads(txn.read_text()).get("status")
                except (OSError, ValueError):
                    state = state or "unreadable"
            row = {"state": state}
            for path in self.files:
                try:
                    row[str(path)] = sha256(path)
                except OSError:
                    row[str(path)] = "missing"
            self.samples.append(row)
            time.sleep(0.02)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join()

    def versions(self, path, states):
        return {row[str(path)] for row in self.samples if row["state"] in states}

    def states(self):
        seen = []
        for row in self.samples:
            if row["state"] and (not seen or seen[-1] != row["state"]):
                seen.append(row["state"])
        return seen


# ----- the host ----------------------------------------------------------------------------------

class Host(u.Host):
    def __init__(self, work, site, netns):
        super().__init__(work, site, netns)
        self.group_upstream = base.free_port(socket.SOCK_DGRAM)

    def http(self, method, path, body=None, timeout=15):
        """u.Host.http with an explicit JSON content type (the /plugins/switchN/post route of v0.2.2 needs it)."""
        request = urllib.request.Request(f"http://127.0.0.1:{self.api}{path}", method=method,
                                         data=None if body is None else json.dumps(body).encode(),
                                         headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(request, timeout=timeout) as reply:
                code, raw = reply.status, reply.read()
        except urllib.error.HTTPError as error:
            code, raw = error.code, error.read()
        try:
            return code, json.loads(raw or b"null")
        except ValueError:
            return code, raw.decode(errors="replace")

    def write_state_file(self, schema, version):
        """The package state in the shape and key order of a deployed root (the program's own serialisation)."""
        (self.root / "webinfo").mkdir(parents=True, exist_ok=True)
        (self.root / "webinfo/rust_config_update_state.json").write_text(json.dumps(
            {"format": 1, "applied_schema": schema, "required_schema": schema, "status": "success",
             "package_id": f"rust-config-schema-{schema}", "package_sha256": "0" * 64, "binary_version": version,
             "files_updated": 18, "backup_dir": "update/txn-seed", "message": "seed",
             "updated_at": "2026-10-23T00:00:00Z"}, indent=2))

    def prepare_synthetic(self, config_tar):
        shutil.rmtree(self.root, ignore_errors=True)
        self.root.mkdir(parents=True)
        run(["tar", "-xf", config_tar, "-C", self.root])
        self.write_state_file(2, "v0.2.2")
        entry = {"tag": "mock", "enabled": True, "protocol": "udp", "addr": f"127.0.0.1:{self.upstream}", "use_proxy": False}
        settings = {"version": 1, "proxies": {"main_proxy": {"addr": "192.0.2.9:7890"}},
                    "upstreams": {group: [dict(entry, tag=f"mock-{group}")] for group in u.GROUPS}}
        (self.root / "webinfo/settings.json").write_text(json.dumps(settings, indent=2, sort_keys=True) + "\n")
        with open(self.root / "rule/whitelist.txt", "a") as rules:
            rules.write(USER_RULE)
        (self.root / "webinfo/appearance_settings.json").write_text('{"theme": "drill", "opacity": 0.5}\n')
        (self.root / "webinfo/appearance_text_settings.json").write_text('{"color": "#123456"}\n')
        (self.root / "webinfo/appearance_button_settings.json").write_text('{"color": "#654321"}\n')
        (self.root / "webinfo/panel_background.bin").write_bytes(bytes(range(256)) * 8)

    def prepare_prodcopy(self, source):
        """A working copy of the deployed root; every group of the settings' upstreams section (and the five ordinary ones)
        gets the fake upstream as its only entry, so nothing needs the network."""
        shutil.rmtree(self.root, ignore_errors=True)
        self.root.parent.mkdir(parents=True, exist_ok=True)
        run(["cp", "-a", source, self.root])
        port_file = self.root / "webinfo/webui_port_settings.json"
        if port_file.exists():   # the deployed API/WebUI port replaces the one of config_custom.yaml
            self.api = int(json.loads(port_file.read_text())["port"])
        path = self.root / "webinfo/settings.json"
        settings = json.loads(path.read_text())
        entry = {"tag": "mock", "enabled": True, "protocol": "udp", "addr": f"127.0.0.1:{self.upstream}", "use_proxy": False}
        settings["upstreams"] = {group: [dict(entry, tag=f"mock-{group}")] for group in sorted(set(settings.get("upstreams", {})) | set(u.GROUPS))}
        path.write_text(json.dumps(settings, indent=2, sort_keys=True) + "\n")

    def runtime_names(self):
        """File names under cache/ and gen/ (runtime state: contents change with every save)."""
        return sorted(str(p.relative_to(self.root)) for d in ("cache", "gen") if (self.root / d).exists()
                      for p in (self.root / d).rglob("*") if p.is_file())

    def switches(self):
        out = {}
        for n in range(1, 18):
            code, body = self.http("GET", f"/plugins/switch{n}/show")
            out[n] = body.strip() if code == 200 and isinstance(body, str) else f"HTTP {code}"
        return out


def settings_without_flags(path):
    data = json.loads(Path(path).read_text())
    return {k: v for k, v in data.items() if k != "flags"}


def broken_candidate(work, repo, config_zip):
    broken = work / "broken"
    broken.mkdir(exist_ok=True)
    script = broken / "atomdns"
    script.write_text("#!/bin/sh\ncase \"$1\" in\n  version) echo v0.3.0;;\n  check) exit 0;;\n  start) exit 3;;\nesac\n")
    script.chmod(0o755)
    env = dict(os.environ, PREBUILT=str(script), RUST_CONFIG_PACKAGE=str(config_zip))
    run([Path(repo) / "scripts/build-rust-release.sh", "v0.3.0", broken / "release"], env=env)
    return broken / "release"


# ----- the drill ---------------------------------------------------------------------------------

def drill(args):
    synthetic = args.mode == "synthetic"
    work = Path(args.work)
    site = u.Site()
    host = Host(work, site, args.netns)
    upstream = start_upstream(host.upstream, ANSWER)
    group_upstream = start_upstream(host.group_upstream, GROUP_ANSWER)
    release, config_zip, v022 = Path(args.release_dir), Path(args.config_zip), Path(args.v022_dir)
    pkg = u.package_files(config_zip)
    state_file = host.root / "webinfo/rust_config_update_state.json"
    settings_file = host.root / "webinfo/settings.json"

    # --- 0 inputs -----------------------------------------------------------------------------
    old_manifest = json.loads((v022 / MANIFEST).read_text())
    new_manifest = json.loads((release / MANIFEST).read_text())
    say("0 v0.2.2 assets", sha256(v022 / ASSET) == old_manifest["artifacts"][ASSET]["sha256"]
        and old_manifest["version"] == "v0.2.2" and old_manifest["required_config_schema"] == 2,
        f"program package sha256 {sha256(v022 / ASSET)[:12]} equals the release manifest's")
    say("0 v0.3.0 manifest", new_manifest["version"] == "v0.3.0" and new_manifest["required_config_schema"] == 3
        and new_manifest["config_package_id"] == "rust-config-schema-3"
        and new_manifest["config"]["sha256"] == sha256(config_zip)
        and new_manifest["config"]["url"].endswith("/releases/download/v0.3.0/config_rust_up.zip"),
        f"config {new_manifest['config']['url']} sha256 {new_manifest['config']['sha256'][:12]} == the package file")
    say("0 package manifest", pkg["config_schema"] == 3 and pkg["package_id"] == "rust-config-schema-3"
        and "sub_config/flags.yaml" in pkg["managed_files"] and "sub_config/special_groups.yaml" not in pkg["managed_files"]
        and pkg["delete_files"] == ["sub_config/switch.yaml"] and not pkg["create_if_missing"],
        f"config_schema 3, {len(pkg['managed_files'])} managed files, flags.yaml in, special_groups.yaml out, "
        f"delete_files {pkg['delete_files']}, create_if_missing empty")
    flags_yaml = zipped_text(config_zip, "sub_config/flags.yaml")
    declarations = flag_declarations(flags_yaml)
    ttls = learner_ttls(zipped_text(config_zip, "sub_config/route_learner.yaml"))
    say("0 flag declarations", sorted(declarations) == sorted(FLAG_KEYS), f"{len(declarations)} feature_flag declarations in the package")

    program_022 = work / "unpacked-v0.2.2"
    program_022.mkdir(parents=True)
    run(["tar", "-xzf", v022 / ASSET, "-C", program_022])
    sha_022 = sha256(program_022 / "atomdns")
    program_030 = work / "unpacked-v0.3.0"
    program_030.mkdir()
    run(["tar", "-xzf", release / ASSET, "-C", program_030])
    sha_030 = sha256(program_030 / "atomdns")
    for path, tag in ((program_022 / "atomdns", "v0.2.2"), (program_030 / "atomdns", "v0.3.0")):
        say(f"0 {tag} binary", run([path, "version"]).strip() == tag, f"reports {tag}")

    host.install_unit()
    shutil.copy2(program_022 / "atomdns", host.exe)
    managed = pkg["managed_files"]
    host.prepare_synthetic(args.config_tar) if synthetic else host.prepare_prodcopy(args.prod_copy)
    expected = expected_flags(host.root, declarations)
    expected_changes = {k: v for k, v in expected.items() if v != declarations[k][1]}
    host.systemctl("start")
    health = host.wait_ready("v0.2.2", 180)

    # --- start v0.2.2 and create the user data ---------------------------------------------------
    probes = [u.query(host.dns, name, 1, timeout=5) for name in ("drill.example.com", "www.example.org", "example.net")]
    say("S start v0.2.2", health["version"] == "v0.2.2" and health.get("applied_config_schema") == 2
        and health.get("required_config_schema") == 2 and dns_query(host.dns) == ANSWER,
        f"version={health['version']} applied_config_schema={health.get('applied_config_schema')} DNS answers {ANSWER} "
        f"(rcode/answer counts of three example names: {probes})")
    if synthetic:
        code, _ = host.http("POST", "/api/v1/special-groups", {"name": GROUP_NAME, "listen_port": GROUP_PORT, "upstreams": [
            {"tag": "drill-owned", "enabled": True, "protocol": "udp", "addr": f"127.0.0.1:{host.group_upstream}",
             "use_socks_proxy": False}]})
        say("S special group created", code == 200, f"POST /api/v1/special-groups (listen_port {GROUP_PORT}) -> {code}")
        host.wait_ready("v0.2.2", 60)
        code, _ = host.http("POST", "/plugins/special_manual_50/post", {"values": GROUP_LIST})
        time.sleep(1)
        groups = host.http("GET", "/api/v1/special-groups")[1]
        routed = dns_query(host.dns, "before1.grp.example.com") == GROUP_ANSWER
        ported = dns_query(GROUP_PORT, "before2.example.com") == GROUP_ANSWER
        say("S special group state", code in (200, 204) and any(g.get("name") == GROUP_NAME for g in groups)
            and (host.root / "webinfo/special_upstream_groups.json").exists() and routed and ported,
            f"group listed with its list and port; a listed name answers {GROUP_ANSWER} on the main port: {routed}, "
            f"any name on port {GROUP_PORT}: {ported}")
        for name in LEARN:
            rcode, records = u.query(host.dns, name, 28)
            assert rcode == 0 and records == 0, (name, rcode, records)
        time.sleep(0.5)
        shown = [c for c in CATEGORIES if all(n in (host.http("GET", f"/plugins/{c}/show")[1] or "") for n in LEARN)]
        say("S learned by v0.2.2", shown == ["my_nov6list"], f"{len(LEARN)} AAAA names learned into {shown}")
        for n, value in ((3, "B"), (12, "A"), (16, "A"), (6, "A")):
            code, _ = host.http("POST", f"/plugins/switch{n}/post", {"value": value})
            assert code == 200, (n, code)
        say("S switches set", [(host.root / f"rule/switch{n}.txt").read_text() for n in (3, 12, 16, 6)] == ["B", "A", "A", "A"],
            "switch3=B switch12=A switch16=A switch6=A written by the v0.2.2 program")
        host.systemctl("stop")      # the normal stop writes gen/route_learner.json
        say("S learner saved", names_of(host.root / "gen/route_learner.json").get("my_nov6list") == sorted(LEARN),
            f"gen/route_learner.json holds the {len(LEARN)} learned names")
        host.systemctl("start")
        host.wait_ready("v0.2.2", 120)
        expected = expected_flags(host.root, declarations)
        expected_changes = {k: v for k, v in expected.items() if v != declarations[k][1]}
        say("S expected migration", expected_changes == EXPECTED_SYNTHETIC_FLAGS,
            f"the table applied to the switch files gives {len(expected_changes)} non-default flags, equal to the planned set")
    else:
        groups = host.http("GET", "/api/v1/special-groups")[1]
        learner = counts_of(host.root / "gen/route_learner.json")
        say("S production-copy start state", isinstance(groups, list) and dns_query(host.dns) == ANSWER
            and (host.root / "webinfo/special_upstream_groups.json").exists()
            and host.http("GET", "/plugins/route_learner/status")[0] == 200,
            f"v0.2.2 healthy, DNS answers, {len(groups)} special groups listed, learned entries {sum(learner.values())} "
            f"in {len(learner)} categories, expected non-default flags by the table: {len(expected_changes)}")
    pid = host.props()["MainPID"]
    before = host.tree()
    before_runtime = host.runtime_names()
    learner_before = counts_of(host.root / "gen/route_learner.json")
    names_before = names_of(host.root / "gen/route_learner.json")
    live_before = must_keep(host.root / "gen/route_learner.json", ttls)
    user_keys = [k for k in before if k.startswith(("rule/", "srs/", "adguard/", "ui/")) or k.startswith("webinfo/")
                 and k != "webinfo/rust_config_update_state.json"]
    user_before = {k: before[k] for k in user_keys}
    state_before = state_file.read_bytes()
    settings_before = settings_file.read_bytes()
    switches_before = host.switches()
    groups_before = host.http("GET", "/api/v1/special-groups")[1]
    json_before = json.loads((host.root / "webinfo/special_upstream_groups.json").read_text())
    root_page_before = urllib.request.urlopen(f"http://127.0.0.1:{host.api}/", timeout=10).read()
    say("S baseline", json.loads(state_before)["applied_schema"] == 2 and "flags" not in json.loads(settings_before)
        and (host.root / "sub_config/switch.yaml").exists() and not (host.root / "sub_config/flags.yaml").exists(),
        f"{len(before)} files hashed; {len(user_before)} user-data files; {len(groups_before)} special groups; "
        f"learned entries {sum(learner_before.values())}; applied_schema 2, no flags section, switch.yaml present")

    # --- 2 configuration package sha256 refused (synthetic only) ------------------------------------
    if synthetic:
        site.publish_release("v0.3.0", release, config_zip, config_sha="0" * 64)
        status = host.check()
        assert status["latest_version"] == "v0.3.0" and status["update_available"] is True, status
        update_before = host.update_dir_names()
        served_before = site.config_downloads("v0.3.0")
        code, reply = host.http("POST", "/api/v1/update/apply", {})
        time.sleep(2)
        props, now = host.props(), host.tree()
        say("2 wrong config sha256 refused", code == 502 and sha256(host.exe) == sha_022 and now == before
            and state_file.read_bytes() == state_before and settings_file.read_bytes() == settings_before
            and host.update_dir_names() == update_before and host.health()["version"] == "v0.2.2"
            and props["MainPID"] == pid and props["NRestarts"] == "0",
            f"{code} {reply}; config package downloaded {site.config_downloads('v0.3.0') - served_before}x; program, "
            f"{len(now)} files, package state and settings.json unchanged, update/ {update_before}, MainPID {props['MainPID']}")

    # --- 3 unhealthy candidate rolled back ------------------------------------------------------------
    broken = broken_candidate(work, args.repo, config_zip)
    site.publish_release("v0.3.0", broken, config_zip)
    status = host.check()
    assert status["latest_version"] == "v0.3.0" and status["update_available"] is True, status
    watched = [host.root / "config_custom.yaml", host.root / "sub_config/flags.yaml", host.root / "sub_config/switch.yaml",
               state_file, settings_file, host.exe]
    with u.Watcher(watched) as watcher:
        code, reply = host.http("POST", "/api/v1/update/apply", {})
        say("3 apply accepted", code == 200, f"{code}")
        health = wait_for("rollback reported", 180, lambda: (lambda h: h if h and h.get("update", {}).get("state") == "rolled_back"
                                                           and h.get("version") == "v0.2.2" and h.get("ready") else None)(host.health()))
    props, now = host.props(), host.tree()
    state_now = json.loads(state_file.read_text())
    seen = watcher.seen
    say("3 rolled back: files", now == before and sha256(host.exe) == sha_022 and state_file.read_bytes() == state_before
        and state_now["applied_schema"] == 2 and settings_file.read_bytes() == settings_before,
        f"all {len(now)} files identical to before (differences: {u.differences(before, now) or 'none'}); program sha256 == v0.2.2; "
        f"package state applied_schema {state_now['applied_schema']}; settings.json byte-identical")
    say("3 trial really changed the files", len(seen[host.root / "config_custom.yaml"]) > 1
        and seen[host.root / "sub_config/flags.yaml"] != {"missing"} and "missing" in seen[host.root / "sub_config/switch.yaml"]
        and len(seen[state_file]) > 1 and len(seen[settings_file]) == 1,
        f"during the trial config_custom.yaml had {len(seen[host.root / 'config_custom.yaml'])} versions, flags.yaml appeared="
        f"{seen[host.root / 'sub_config/flags.yaml'] != {'missing'}}, switch.yaml disappeared={'missing' in seen[host.root / 'sub_config/switch.yaml']}, "
        f"package state versions {len(seen[state_file])}, settings.json versions {len(seen[settings_file])}")
    status = host.check()
    learned_ok, learned_added, learned_gone = kept(live_before, names_before, names_of(host.root / "gen/route_learner.json"))
    parts = {"MainPID unchanged": props["MainPID"] == pid, "NRestarts 0": props["NRestarts"] == "0",
             "rollback_performed": status.get("rollback_performed") is True, "DNS answers": dns_query(host.dns) == ANSWER,
             "flags.yaml removed": not (host.root / "sub_config/flags.yaml").exists(),
             "switch.yaml restored": (host.root / "sub_config/switch.yaml").exists(),
             "17 switch values equal": host.switches() == switches_before,
             "cache/gen names unchanged": host.runtime_names() == before_runtime,
             "learned names kept": learned_ok}
    say("3 rolled back: service", all(parts.values()),
        f"MainPID {props['MainPID']} (unchanged), NRestarts {props['NRestarts']}, DNS {ANSWER}, flags.yaml removed again, "
        f"switch.yaml restored, 17 switch values equal to before, cache/ and gen/ file names unchanged "
        f"({len(before_runtime)}), learned entries {sum(learner_before.values())} still there except {learned_gone} expired ones (+{learned_added} learned from probe queries), reason {health['update'].get('last_error')!r}"
        + "".join(f"; NOT OK: {name}" for name, ok in parts.items() if not ok))

    # --- 1 success path -----------------------------------------------------------------------------------
    site.publish_release("v0.3.0", release, config_zip)
    status = host.check()
    say("1 check", status["latest_version"] == "v0.3.0" and status["update_available"] is True
        and status["current_version"] == "v0.2.2" and status.get("config_schema_applied") == 2,
        f"current={status['current_version']} latest={status['latest_version']} config_schema_applied={status.get('config_schema_applied')}")
    served_before = site.config_downloads("v0.3.0")
    with Timeline(host.root, [settings_file]) as timeline, base.Prober(host.dns) as probe:
        code, reply = host.http("POST", "/api/v1/update/apply", {})
        say("1 apply accepted", code == 200 and isinstance(reply, dict) and reply.get("restart_required") is True,
            f"{code} {reply if code != 200 else reply.get('notes', '')}")
        health = host.wait_ready("v0.3.0", 180)
        wait_for("update transaction closed", 120, lambda: host.health()["update"]["state"] == "idle")
        time.sleep(1.5)
    props = host.props()
    installed = json.loads((host.root / "update/installed.json").read_text())
    state = json.loads(state_file.read_text())
    health = host.health()
    say("1 upgraded", sha256(host.exe) == sha_030 and health["version"] == "v0.3.0"
        and health.get("applied_config_schema") == 3 and health.get("required_config_schema") == 3
        and state["applied_schema"] == 3 and state["package_id"] == "rust-config-schema-3" and state["status"] == "success"
        and installed["version"] == "v0.3.0" and props["MainPID"] == pid and props["NRestarts"] == "0"
        and dns_query(host.dns) == ANSWER and site.config_downloads("v0.3.0") - served_before == 1,
        f"program sha256 == v0.3.0 build; health version {health['version']} applied_config_schema {health.get('applied_config_schema')}; "
        f"package state applied_schema {state['applied_schema']} package_id {state['package_id']}; MainPID {props['MainPID']} (unchanged), "
        f"NRestarts {props['NRestarts']}; config package downloaded once; DNS during the update: {probe.summary()}")
    trial_states = ("installing", "testing")
    say("1 trial leaves settings.json alone", "testing" in timeline.states()
        and timeline.versions(settings_file, trial_states) == {sha256_bytes(settings_before)}
        and settings_file.read_bytes() != settings_before,
        f"transaction states seen {timeline.states()}; settings.json had 1 version while installing/testing (the schema 2 bytes); "
        "the final program then wrote the migration")
    after = host.tree()
    want = pkg["sha256"]
    wrong = [rel for rel in managed if after.get(rel) != want[rel]]
    allowed = set(managed) | {"webinfo/rust_config_update_state.json", "webinfo/settings.json", "sub_config/special_groups.yaml",
                              "sub_config/switch.yaml"}
    differing = set(u.differences(before, after))
    say("1 managed files are the package's", not wrong and "sub_config/flags.yaml" in after
        and "sub_config/switch.yaml" not in after and differing <= allowed,
        f"all {len(managed)} managed files equal the package manifest's sha256; switch.yaml deleted, flags.yaml present; "
        f"{len(differing)} files differ from before, all of them managed, generated (special_groups.yaml), the package state or settings.json")
    user_after = {k: after.get(k) for k in user_keys}
    changed_user = sorted(k for k in user_keys if user_after[k] != user_before[k])
    say("1 user data kept", changed_user == ["webinfo/settings.json"] and all(
        after.get(f"rule/switch{n}.txt") == before.get(f"rule/switch{n}.txt") for n in range(1, 18)),
        f"{len(user_keys) - 1} of {len(user_keys)} user-data files (rule/ incl. 17 switch files, srs/, adguard/, ui/, webinfo/) "
        "byte-identical; only webinfo/settings.json differs (the flags section)")
    saved = json.loads(settings_file.read_text())
    say("1 settings.json: only flags added", settings_without_flags(settings_file) == json.loads(settings_before)
        and saved.get("flags") == expected_changes
        and (not synthetic or (saved["flags"] == EXPECTED_SYNTHETIC_FLAGS and saved["proxies"]["main_proxy"]["addr"] == "192.0.2.9:7890")),
        f"every other section equal to before; flags section has {len(saved.get('flags', {}))} entries equal to the independently computed "
        f"{len(expected_changes)}" + ("; equals {core_mode: safe, client_routing: listed_direct, ddns_foreign: true, block_aaaa: true}; proxy override kept"
                                     if synthetic else ""))
    code, flags = host.http("GET", "/api/v1/flags")
    rows = {row["key"]: row for row in flags["flags"]} if code == 200 else {}
    sources_ok = all(row["source"] == ("settings" if row["key"] in expected_changes else "yaml") for row in rows.values())
    say("1 /api/v1/flags", code == 200 and flags["migrated"] is True and sorted(rows) == sorted(FLAG_KEYS)
        and all(rows[k]["value"] == expected[k] for k in FLAG_KEYS) and sources_ok,
        f"{len(rows)} flags, migrated true, every value equal to the independent computation, source settings for the "
        f"{len(expected_changes)} non-default ones and yaml for the rest")
    groups = host.http("GET", "/api/v1/special-groups")[1]
    json_after = json.loads((host.root / "webinfo/special_upstream_groups.json").read_text())
    generated = (host.root / "sub_config/special_groups.yaml").read_text()
    learner_after = counts_of(host.root / "gen/route_learner.json")
    status_code = host.http("GET", "/plugins/route_learner/status")[0]
    learned_ok, _, learned_gone = kept(live_before, names_before, names_of(host.root / "gen/route_learner.json"))
    parts = {"group count equal": len(groups) == len(groups_before), "special_upstream_groups.json equal": json_after == json_before,
             "route_learner/status 200": status_code == 200, "live learned names kept": learned_ok,
             "generated YAML: no switchN": not re.search(r"switch\d", generated),
             "generated YAML: flag lazy_cache when groups exist": not groups or "flag lazy_cache" in generated}
    say("1 special groups", all(parts.values()),
        f"{len(groups)} special groups listed (before {len(groups_before)}); special_upstream_groups.json equal; "
        f"special_groups.yaml ({len(generated)} bytes) has no switchN reference" + ("; uses flag lazy_cache" if groups else "")
        + f"; gen/route_learner.json: {sum(learner_before.values())} entries before, {sum(learner_after.values())} now ({learned_gone} expired "
        f"ones swept, the live ones kept); /plugins/route_learner/status {status_code}"
        + "".join(f"; NOT OK: {name}" for name, ok in parts.items() if not ok))
    check = run([program_030 / "atomdns", "check", "-d", host.root, "-c", host.root / "config_custom.yaml"])
    say("1 check is clean", bool(re.search(r"gaps=0\b.*cascades=0\b.*globals=0\b", check, re.S)),
        "atomdns check on the updated root reports gaps=0 cascades=0 globals=0")
    if synthetic:
        group_ok = dns_query(host.dns, "after1.grp.example.com") == GROUP_ANSWER and dns_query(GROUP_PORT, "after2.example.com") == GROUP_ANSWER
        other = dns_query(host.dns, "other.example.com") == ANSWER
        say("1 group list and port work", group_ok and other and any(g.get("name") == GROUP_NAME for g in groups),
            f"a new name under the group's list answers {GROUP_ANSWER} on the main port, any name on port {GROUP_PORT} answers {GROUP_ANSWER}, "
            f"other names still answer {ANSWER}")
        say("1 learning kept", names_of(host.root / "gen/route_learner.json") == names_before,
            f"gen/route_learner.json names equal to before ({len(LEARN)} learned names still listed)")
        page = urllib.request.urlopen(f"http://127.0.0.1:{host.api}/", timeout=10).read()
        assets = sorted(set(re.findall(rb'(?:src|href)="(/?assets/[^"]+)"', page)))
        fetched = [urllib.request.urlopen(f"http://127.0.0.1:{host.api}/{a.decode().lstrip('/')}", timeout=10).status for a in assets]
        say("1 new web UI", page != root_page_before and assets and all(code == 200 for code in fetched),
            f"GET / answers a different page than before ({len(page)} vs {len(root_page_before)} bytes); {len(assets)} referenced assets, all 200")
        status_before = host.http("GET", "/plugins/requery/status")[1]
        code, reply = host.http("POST", "/api/v1/flags/dns_mode", {"value": "realip"})
        ran = wait_for("requery task", 20, lambda: (lambda s: s if s.get("task_state") == "running"
                       or s.get("status", {}).get("task_state") == "running" or s != status_before else None)(
                           host.http("GET", "/plugins/requery/status")[1]))
        say("1 dns_mode change", code == 200 and reply.get("requery_triggered") is True and bool(ran),
            f"POST /api/v1/flags/dns_mode realip -> {code} requery_triggered {reply.get('requery_triggered')}; requery status changed (task started)")
        host.systemctl("stop")
        saved_names = names_of(host.root / "gen/route_learner.json")
        say("1 stop keeps the learned names", all(set(rows) <= set(saved_names.get(c, [])) for c, rows in names_before.items()),
            f"after a normal stop gen/route_learner.json still holds every name learned by v0.2.2 ({sum(map(len, names_before.values()))}); "
            f"{sum(map(len, saved_names.values()))} names in all (the requery that the dns_mode change started may have added more)")
        host.systemctl("start")
        host.wait_ready("v0.3.0", 120)
        shown = host.http("GET", "/plugins/my_nov6list/show")[1]
        say("1 restart restores", all(n in shown for n in LEARN) and dns_query(host.dns) == ANSWER,
            "after start the learned names are listed again; DNS answers")
    else:
        host.systemctl("stop")
        after_stop = counts_of(host.root / "gen/route_learner.json")
        ok, added, gone = kept(live_before, names_before, names_of(host.root / "gen/route_learner.json"))
        say("1 stop keeps the learned entries", ok,
            f"after a normal stop gen/route_learner.json has {sum(after_stop.values())} entries in {len(after_stop)} categories "
            f"(before: {sum(learner_before.values())} in {len(learner_before)}); every earlier one still live for 30 min is there ({gone} expired ones swept), +{added} from probe queries")
        host.systemctl("start")
        host.wait_ready("v0.3.0", 180)
        say("1 restart", dns_query(host.dns) == ANSWER and host.http("GET", "/api/v1/flags")[0] == 200, "after start DNS answers and /api/v1/flags is 200")
    pid = host.props()["MainPID"]

    # --- 4 after the update (synthetic only) ------------------------------------------------------------------
    if synthetic:
        status = host.check()
        say("4 no update", status["current_version"] == "v0.3.0" and status["update_available"] is False
            and status.get("config_schema_applied") == 3,
            f"current={status['current_version']} available={status['update_available']} config_schema_applied=3")
        code, reply = host.http("POST", "/api/v1/update/apply", {})
        say("4 apply without force", code == 200 and reply["installed"] is False and reply["restart_required"] is False,
            f"{code} installed=false restart_required=false")
        served_before = site.config_downloads("v0.3.0")
        tree_before = host.tree()
        flags_before = json.loads(settings_file.read_text())["flags"]
        code, reply = host.http("POST", "/api/v1/update/apply", {"force": True})
        say("4 force accepted", code == 200 and reply["restart_required"] is True, f"{code}")
        time.sleep(3)
        host.wait_ready("v0.3.0", 120)
        wait_for("update transaction closed", 120, lambda: host.health()["update"]["state"] == "idle")
        props = host.props()
        say("4 force reinstalled", sha256(host.exe) == sha_030 and props["MainPID"] == pid and props["NRestarts"] == "0"
            and site.config_downloads("v0.3.0") == served_before and host.tree() == tree_before
            and json.loads(settings_file.read_text())["flags"] == flags_before and flags_before.get("dns_mode") == "realip"
            and dns_query(host.dns) == ANSWER,
            f"same program, MainPID {props['MainPID']} (unchanged), NRestarts {props['NRestarts']}; config package not downloaded again "
            f"({site.config_downloads('v0.3.0')} downloads in the whole drill: refused sha, rolled-back attempt, success); all {len(tree_before)} "
            f"files unchanged; settings.json flags ({len(flags_before)} entries, incl. the dns_mode change made after the update) not migrated again")
    upstream.close()
    group_upstream.close()


def sha256_bytes(data):
    return base.sha256(data)


def zipped_text(path, name):
    import zipfile
    with zipfile.ZipFile(path) as package:
        return package.read(name).decode()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--mode", choices=("synthetic", "prodcopy"), required=True)
    parser.add_argument("--work", required=True, help="empty directory for this drill (removed at the end)")
    parser.add_argument("--repo", required=True, help="the repository checkout holding scripts/")
    parser.add_argument("--v022-dir", required=True)
    parser.add_argument("--release-dir", required=True)
    parser.add_argument("--config-zip", required=True)
    parser.add_argument("--config-tar")
    parser.add_argument("--prod-copy")
    parser.add_argument("--keep", action="store_true", help="keep the work directory")
    parser.add_argument("--log", help="also write the STEP lines to this file")
    parser.add_argument("--netns", default="rtdrill")
    args = parser.parse_args()
    if os.geteuid() != 0:
        raise SystemExit("run as root on the test VM")
    if (args.mode == "synthetic") != bool(args.config_tar) or (args.mode == "prodcopy") != bool(args.prod_copy):
        raise SystemExit("synthetic needs --config-tar, prodcopy needs --prod-copy (and neither takes the other's)")
    work = Path(args.work).resolve()
    if os.environ.get(u.NS_ENV) != args.netns:
        if work.exists() and any(work.iterdir()):
            raise SystemExit(f"{work} is not empty")
        base.refuse_existing_unit()
        if Path(f"/run/netns/{args.netns}").exists():
            raise SystemExit(f"network namespace {args.netns} already exists; refusing to touch it")
        work.mkdir(parents=True, exist_ok=True)
        code = 1
        try:
            u.prepare_namespace(args.netns)
            argv = ["ip", "netns", "exec", args.netns, sys.executable, *sys.argv]
            code = subprocess.run(argv, env=dict(os.environ, **{u.NS_ENV: args.netns})).returncode
        finally:
            subprocess.run(["ip", "netns", "delete", args.netns], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            if not args.keep:
                shutil.rmtree(work, ignore_errors=True)
        raise SystemExit(code)
    args.work = str(work)
    try:
        drill(args)
        print("DRILL PASSED", flush=True)
    finally:
        try:
            if args.log:
                Path(args.log).write_text("\n".join(base.RESULTS) + "\n")
        finally:
            base.cleanup(work, True)


if __name__ == "__main__":
    main()
