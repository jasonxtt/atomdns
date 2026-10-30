#!/usr/bin/env python3
"""VM-only drill of the online update v0.2.1 -> v0.2.2 with a configuration package (schema 1 -> 2)
(task 10-23-atomdns-release-v0.2.2).

Starts the REAL v0.2.1 release program as a systemd unit named `atomdns` (Restart=always,
RestartSec=120) on the `config_rust` tree of the file repository at 0412666 (schema 1) plus the
package state of the production root, with user data (proxy override, upstream overrides to a fake
upstream, a special group made through the API, a user rule and a switch file). A fake GitHub API on
127.0.0.1 (ATOMDNS_UPDATE_API_URL) offers the v0.2.2 candidate and the configuration package.
Everything runs in a loopback-only network namespace created and removed by this script: no public
network, no clash with other services of the machine, only RFC 5737/3849 addresses.

    update_drill.py --work DIR --repo DIR --v021-dir DIR --release-dir DIR --config-zip FILE --config-tar FILE

  --v021-dir     the v0.2.1 release assets (atomdns-linux-amd64.tar.gz + atomdns-update-manifest.json)
  --release-dir  the v0.2.2 output of scripts/build-rust-release.sh (with RUST_CONFIG_PACKAGE)
  --config-zip   the config_rust_up.zip that the metadata names (schema 2)
  --config-tar   the schema 1 config_rust tree as a tar (file repository 0412666)

Scenarios: 2 config package sha256 refused, 3 unhealthy candidate rolled back, 1 success path
(learning, data kept, stop writes gen/route_learner.json), 4 no update / force. Needs root, systemd,
iproute2, jq and tar. Removes the unit, the namespace, the processes and (unless --keep) the work
directory when it ends, whatever the outcome.
"""
import argparse
import hashlib
import importlib.util
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("rename_drill", HERE.parent / "rename-drill/drill.py")
base = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(base)

say, sha256, run, wait_for, dns_query = base.say, base.sha256, base.run, base.wait_for, base.dns_query
ASSET, MANIFEST, ANSWER = base.ASSET, base.MANIFEST, base.ANSWER
CONFIG_ASSET = "config_rust_up.zip"
GROUPS = ("domestic", "foreign", "foreignecs", "nocnfake", "cnfake")
NS_ENV = "RTDRILL_NETNS"
LEARN_AAAA = "learn6.example.com"
GROUP_NAME = "drillgrp"
USER_RULE = "full:userrule.example.com\n"


class CountingFiles(dict):
    """The fake site's file table; counts how often each path was served."""

    def __init__(self):
        super().__init__()
        self.served = {}

    def __getitem__(self, key):
        self.served[key] = self.served.get(key, 0) + 1
        return super().__getitem__(key)


class Site(base.Site):
    def __init__(self):
        super().__init__()
        self.files = CountingFiles()

    def publish_release(self, tag, directory, config_zip, config_sha=None):
        """The release's program package and manifest plus the configuration package as assets;
        the manifest's config.url points at this fake site (the real one names github.com)."""
        manifest = json.loads(Path(directory, MANIFEST).read_text())
        manifest["config"] = {"url": f"http://127.0.0.1:{self.port}/dl/{tag}/{CONFIG_ASSET}",
                              "sha256": config_sha or sha256(config_zip)}
        self.files[f"/dl/{tag}/{ASSET}"] = Path(directory, ASSET).read_bytes()
        self.files[f"/dl/{tag}/{MANIFEST}"] = json.dumps(manifest).encode()
        self.files[f"/dl/{tag}/{CONFIG_ASSET}"] = Path(config_zip).read_bytes()
        self.add(tag, False, False, [ASSET, MANIFEST, CONFIG_ASSET])

    def config_downloads(self, tag):
        return self.files.served.get(f"/dl/{tag}/{CONFIG_ASSET}", 0)


class Host(base.Host):
    """Production-like ports inside the private namespace: DNS on 53, API on 9099."""

    def __init__(self, work, site, netns):
        super().__init__(work, site)
        self.api, self.dns = 9099, 53
        self.netns = netns

    def install_unit(self):
        base.refuse_existing_unit()
        self.exe.parent.mkdir(parents=True, exist_ok=True)
        base.UNIT_FILE.write_text(
            "[Unit]\nDescription=atomdns release drill (temporary)\nAfter=network.target\n\n"
            "[Service]\nType=simple\nPrivateTmp=true\n"
            f"NetworkNamespacePath=/run/netns/{self.netns}\n"
            f"Environment=ATOMDNS_UPDATE_API_URL={self.site.api_url}\n"
            "Environment=ATOMDNS_UPDATE_HEALTH_TIMEOUT_SECS=60\n"
            f"ExecStart={self.exe} start -d {self.root} -c {self.root}/config_custom.yaml\n"
            "Restart=always\nRestartSec=120\n\n[Install]\nWantedBy=multi-user.target\n")
        base.UNIT_OWNED = True
        run(["systemctl", "daemon-reload"])

    def http(self, method, path, body=None, timeout=15):
        """base.Host.http, but a plain-text answer (the /plugins routes) comes back as text."""
        request = urllib.request.Request(f"http://127.0.0.1:{self.api}{path}", method=method,
                                         data=None if body is None else json.dumps(body).encode())
        try:
            with urllib.request.urlopen(request, timeout=timeout) as reply:
                code, raw = reply.status, reply.read()
        except urllib.error.HTTPError as error:
            code, raw = error.code, error.read()
        try:
            return code, json.loads(raw or b"null")
        except ValueError:
            return code, raw.decode(errors="replace")

    def prepare_root(self, config_tar):
        """The production-shaped schema 1 root and the user data to be kept."""
        shutil.rmtree(self.root, ignore_errors=True)
        self.root.mkdir(parents=True)
        run(["tar", "-xf", config_tar, "-C", self.root])
        # Written the way the updater itself writes it (pretty JSON, files_updated): a rollback
        # re-serialises an existing state file, so only then are the bytes comparable.
        (self.root / "webinfo/rust_config_update_state.json").write_text(json.dumps(
            {"format": 1, "applied_schema": 1, "required_schema": 1, "status": "success",
             "package_id": "rust-config-schema-1", "binary_version": "v0.2.1", "files_updated": 0}, indent=2))
        entry = {"tag": "mock", "enabled": True, "protocol": "udp", "addr": f"127.0.0.1:{self.upstream}"}
        settings = {"version": 1, "proxies": {"main_proxy": {"addr": "192.0.2.9:7890"}},
                    "upstreams": {group: [dict(entry, tag=f"mock-{group}")] for group in GROUPS}}
        (self.root / "webinfo/settings.json").write_text(json.dumps(settings, indent=2, sort_keys=True) + "\n")
        with open(self.root / "rule/whitelist.txt", "a") as rules:
            rules.write(USER_RULE)
        (self.root / "rule/switch10.txt").write_text("A")   # the package default is B: a user's choice

    def tree(self):
        """sha256 of every file of the root except update/, cache/ and gen/ (runtime data)."""
        out = {}
        for path in sorted(self.root.rglob("*")):
            rel = path.relative_to(self.root)
            if rel.parts[0] in ("update", "cache", "gen") or not path.is_file():
                continue
            out[str(rel)] = sha256(path)
        return out


def query(port, name, qtype, timeout=2.0):
    """(rcode, answer count) of one UDP query."""
    labels = b"".join(bytes([len(part)]) + part.encode() for part in name.split("."))
    packet = struct.pack(">HHHHHH", 0x4321, 0x0100, 1, 0, 0, 0) + labels + b"\0" + struct.pack(">HH", qtype, 1)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.settimeout(timeout)
        sock.sendto(packet, ("127.0.0.1", port))
        reply, _ = sock.recvfrom(4096)
    return reply[3] & 0x0F, struct.unpack(">H", reply[6:8])[0]


def differences(before, after):
    return sorted(k for k in set(before) | set(after) if before.get(k) != after.get(k))


class Watcher:
    """Records every distinct version of the given files seen while the update runs."""

    def __init__(self, paths):
        self.paths, self.seen, self.stop = paths, {p: set() for p in paths}, threading.Event()
        self.thread = threading.Thread(target=self.loop, daemon=True)

    def loop(self):
        while not self.stop.is_set():
            for path in self.paths:
                try:
                    self.seen[path].add(sha256(path))
                except OSError:
                    self.seen[path].add("missing")
            time.sleep(0.02)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join()


def package_files(config_zip):
    with zipfile.ZipFile(config_zip) as package:
        manifest = json.loads(package.read("manifest.json"))
    return manifest


def drill(args):
    work = Path(args.work)
    site = Site()
    host = Host(work, site, args.netns)
    upstream = base.start_upstream(host.upstream)
    release = Path(args.release_dir)
    config_zip = Path(args.config_zip)
    v021 = Path(args.v021_dir)
    pkg = package_files(config_zip)

    # --- 0 inputs -----------------------------------------------------------------------------
    old_manifest = json.loads((v021 / MANIFEST).read_text())
    new_manifest = json.loads((release / MANIFEST).read_text())
    say("0 v0.2.1 assets", sha256(v021 / ASSET) == old_manifest["artifacts"][ASSET]["sha256"]
        and old_manifest["version"] == "v0.2.1" and old_manifest["required_config_schema"] == 1,
        f"program package sha256 {sha256(v021 / ASSET)[:12]} equals the release manifest's")
    say("0 v0.2.2 manifest", new_manifest["version"] == "v0.2.2" and new_manifest["required_config_schema"] == 2
        and new_manifest["config_package_id"] == "rust-config-schema-2"
        and new_manifest["config"]["sha256"] == sha256(config_zip)
        and new_manifest["config"]["url"].endswith("/releases/download/v0.2.2/config_rust_up.zip"),
        f"config {new_manifest['config']['url']} sha256 {new_manifest['config']['sha256'][:12]} == the package file")
    say("0 package manifest", pkg["config_schema"] == 2 and pkg["package_id"] == "rust-config-schema-2"
        and "sub_config/route_learner.yaml" in pkg["managed_files"]
        and "sub_config/special_groups.yaml" not in pkg["managed_files"],
        f"config_schema 2, {len(pkg['managed_files'])} managed files, route_learner.yaml in, special_groups.yaml out")

    program_021 = work / "unpacked-v0.2.1"
    program_021.mkdir(parents=True)
    run(["tar", "-xzf", v021 / ASSET, "-C", program_021])
    sha_021 = sha256(program_021 / "atomdns")
    program_022 = work / "unpacked-v0.2.2"
    program_022.mkdir()
    run(["tar", "-xzf", release / ASSET, "-C", program_022])
    sha_022 = sha256(program_022 / "atomdns")
    for path, tag in ((program_021 / "atomdns", "v0.2.1"), (program_022 / "atomdns", "v0.2.2")):
        say(f"0 {tag} binary", run([path, "version"]).strip() == tag, f"reports {tag}")

    host.install_unit()
    host.prepare_root(args.config_tar)
    shutil.copy2(program_021 / "atomdns", host.exe)
    managed = pkg["managed_files"]

    # --- start v0.2.1 and create the user data ---------------------------------------------------
    host.systemctl("start")
    health = host.wait_ready("v0.2.1", 120)
    say("S start v0.2.1", health["version"] == "v0.2.1" and health.get("applied_config_schema") == 1
        and health.get("required_config_schema") == 1 and dns_query(host.dns) == ANSWER,
        f"version={health['version']} applied_config_schema={health.get('applied_config_schema')} DNS answers {ANSWER}")
    code, reply = host.http("POST", "/api/v1/special-groups", {"name": GROUP_NAME, "upstreams": [
        {"tag": "drill-owned", "enabled": True, "protocol": "udp", "addr": f"127.0.0.1:{host.upstream}",
         "use_socks_proxy": False}]})
    say("S special group created", code == 200, f"POST /api/v1/special-groups -> {code}")
    host.wait_ready("v0.2.1", 60)
    code, reply = host.http("POST", "/plugins/special_manual_50/post", {"values": ["full:group.example.com"]})
    time.sleep(1)
    groups = host.http("GET", "/api/v1/special-groups")[1]
    generated = (host.root / "sub_config/special_groups.yaml").read_bytes()
    say("S special group state", code in (200, 204) and any(g.get("name") == GROUP_NAME for g in groups)
        and (host.root / "webinfo/special_upstream_groups.json").exists() and len(generated) > 0,
        f"group listed, special_groups.yaml generated ({len(generated)} bytes), webinfo/special_upstream_groups.json written")
    pid = host.props()["MainPID"]
    before = host.tree()
    user_keys = [k for k in before if k.startswith(("rule/", "srs/", "adguard/")) or k in (
        "webinfo/settings.json", "webinfo/special_upstream_groups.json", "sub_config/special_groups.yaml")]
    user_before = {k: before[k] for k in user_keys}
    state_before = (host.root / "webinfo/rust_config_update_state.json").read_bytes()
    say("S baseline", "rule/switch10.txt" in user_before and (host.root / "rule/switch10.txt").read_text() == "A"
        and USER_RULE.strip() in (host.root / "rule/whitelist.txt").read_text()
        and json.loads(state_before)["applied_schema"] == 1,
        f"{len(before)} files hashed; {len(user_before)} user-data files; MainPID {pid}; applied_schema 1")

    # --- 2 configuration package sha256 refused ------------------------------------------------------
    site.publish_release("v0.2.2", release, config_zip, config_sha="0" * 64)
    status = host.check()
    assert status["latest_version"] == "v0.2.2" and status["update_available"] is True, status
    names_before = host.update_dir_names()
    served_before = site.config_downloads("v0.2.2")
    code, reply = host.http("POST", "/api/v1/update/apply", {})
    time.sleep(2)
    props = host.props()
    now = host.tree()
    say("2 wrong config sha256 refused", code == 502 and sha256(host.exe) == sha_021 and now == before
        and (host.root / "webinfo/rust_config_update_state.json").read_bytes() == state_before
        and host.update_dir_names() == names_before and host.health()["version"] == "v0.2.1"
        and props["MainPID"] == pid and props["NRestarts"] == "0",
        f"{code} {reply}; config package downloaded {site.config_downloads('v0.2.2') - served_before}x; program, "
        f"{len(now)} files and package state unchanged, update/ {names_before}, MainPID {props['MainPID']}")

    # --- 3 unhealthy candidate rolled back ------------------------------------------------------------
    broken = work / "broken"
    broken.mkdir()
    script = broken / "atomdns"
    script.write_text("#!/bin/sh\ncase \"$1\" in\n  version) echo v0.2.2;;\n  check) exit 0;;\n  start) exit 3;;\nesac\n")
    script.chmod(0o755)
    env = dict(os.environ, PREBUILT=str(script), RUST_CONFIG_PACKAGE=str(config_zip))
    run([Path(args.repo) / "scripts/build-rust-release.sh", "v0.2.2", broken / "release"], env=env)
    site.publish_release("v0.2.2", broken / "release", config_zip)
    status = host.check()
    assert status["latest_version"] == "v0.2.2" and status["update_available"] is True, status
    state_file = host.root / "webinfo/rust_config_update_state.json"
    watched = [host.root / "config_custom.yaml", host.root / "sub_config/route_learner.yaml", state_file, host.exe]
    with Watcher(watched) as watcher:
        code, reply = host.http("POST", "/api/v1/update/apply", {})
        say("3 apply accepted", code == 200, f"{code}")
        health = wait_for("rollback reported", 120, lambda: (lambda h: h if h and h.get("update", {}).get("state") == "rolled_back"
                                                           and h.get("version") == "v0.2.1" and h.get("ready") else None)(host.health()))
    props = host.props()
    now = host.tree()
    trial_config = len(watcher.seen[host.root / "config_custom.yaml"]) > 1
    trial_new_file = watcher.seen[host.root / "sub_config/route_learner.yaml"] != {"missing"}
    trial_state = len(watcher.seen[state_file]) > 1
    state_now = json.loads(state_file.read_text())
    say("3 rolled back: files", now == before and sha256(host.exe) == sha_021
        and state_file.read_bytes() == state_before and state_now["applied_schema"] == 1,
        f"all {len(now)} files identical to before (differences: {differences(before, now) or 'none'}); "
        f"program sha256 == v0.2.1; package state applied_schema {state_now['applied_schema']}")
    say("3 trial really changed the files", trial_config and trial_new_file and trial_state,
        f"during the trial config_custom.yaml had {len(watcher.seen[host.root / 'config_custom.yaml'])} versions, "
        f"route_learner.yaml appeared={trial_new_file}, package state versions {len(watcher.seen[state_file])}")
    status = host.check()
    say("3 rolled back: service", props["MainPID"] == pid and props["NRestarts"] == "0"
        and status.get("rollback_performed") is True and dns_query(host.dns) == ANSWER
        and not (host.root / "sub_config/route_learner.yaml").exists(),
        f"MainPID {props['MainPID']} (unchanged), NRestarts {props['NRestarts']}, DNS {ANSWER}, "
        f"route_learner.yaml removed again, reason {health['update'].get('last_error')!r}")

    # --- 1 success path -----------------------------------------------------------------------------------
    site.publish_release("v0.2.2", release, config_zip)
    status = host.check()
    say("1 check", status["latest_version"] == "v0.2.2" and status["update_available"] is True
        and status["current_version"] == "v0.2.1" and status.get("config_schema_applied") == 1,
        f"current={status['current_version']} latest={status['latest_version']} "
        f"config_schema_applied={status.get('config_schema_applied')} (the running program requires "
        f"{status.get('config_schema_required')})")
    served_before = site.config_downloads("v0.2.2")
    with base.Prober(host.dns) as probe:
        code, reply = host.http("POST", "/api/v1/update/apply", {})
        say("1 apply accepted", code == 200 and isinstance(reply, dict) and reply.get("restart_required") is True,
            f"{code} {reply if code != 200 else reply.get('notes', '')}")
        health = host.wait_ready("v0.2.2", 120)
        wait_for("update transaction closed", 90, lambda: host.health()["update"]["state"] == "idle")
        time.sleep(1)
    props = host.props()
    installed = json.loads((host.root / "update/installed.json").read_text())
    state = json.loads(state_file.read_text())
    health = host.health()
    say("1 upgraded", sha256(host.exe) == sha_022 and health["version"] == "v0.2.2"
        and health.get("applied_config_schema") == 2 and health.get("required_config_schema") == 2
        and state["applied_schema"] == 2 and state["package_id"] == "rust-config-schema-2"
        and state["status"] == "success" and installed["version"] == "v0.2.2"
        and props["MainPID"] == pid and props["NRestarts"] == "0" and dns_query(host.dns) == ANSWER
        and site.config_downloads("v0.2.2") - served_before == 1,
        f"program sha256 == v0.2.2 build; health version {health['version']} applied_config_schema "
        f"{health.get('applied_config_schema')}; package state applied_schema {state['applied_schema']} "
        f"package_id {state['package_id']}; MainPID {props['MainPID']} (unchanged), NRestarts {props['NRestarts']}; "
        f"config package downloaded once; DNS during the update: {probe.summary()}")
    after = host.tree()
    want = pkg["sha256"]
    wrong = [rel for rel in managed if after.get(rel) != want[rel]]
    say("1 managed files are the package's", not wrong and "sub_config/route_learner.yaml" in after
        and sorted(differences(before, after)) == sorted(set(differences(before, after)) & (set(managed) | {"webinfo/rust_config_update_state.json"})),
        f"all {len(managed)} managed files equal the package manifest's sha256; files that differ from before the update: "
        f"{', '.join(differences(before, after))}")
    user_after = {k: after.get(k) for k in user_keys}
    say("1 user data kept", user_after == user_before and (host.root / "rule/switch10.txt").read_text() == "A",
        f"{len(user_keys)} user-data files byte-identical: webinfo/settings.json, webinfo/special_upstream_groups.json, "
        f"sub_config/special_groups.yaml (generated), rule/ (user rule line, switch10.txt 'A'), srs/, adguard/")
    groups = host.http("GET", "/api/v1/special-groups")[1]
    effective = host.http("GET", "/api/v1/settings/effective")[1]
    say("1 settings and group live", any(g.get("name") == GROUP_NAME for g in groups)
        and "192.0.2.9:7890" in json.dumps(effective),
        "special group listed by the new program; the proxy override 192.0.2.9:7890 is in the effective settings")

    # learning: an AAAA query answered with an empty NOERROR is learned as "remembered no V6"
    rcode, records = query(host.dns, LEARN_AAAA, 28)
    time.sleep(0.5)
    shown = {}
    for category in ("my_nov6list", "my_nov4list", "my_realiplist", "my_fakeiplist", "my_sv4list"):
        code, body = host.http("GET", f"/plugins/{category}/show")
        shown[category] = body if code == 200 else f"HTTP {code}"
    learned = [c for c, body in shown.items() if isinstance(body, str) and LEARN_AAAA in body]
    say("1 learning visible", rcode == 0 and records == 0 and learned == ["my_nov6list"],
        f"AAAA {LEARN_AAAA} answered empty NOERROR; /plugins/<category>/show lists it in {learned}")
    status = host.http("GET", "/plugins/route_learner/status")[1]
    print(f"INFO route_learner status: {json.dumps(status)[:300]}", flush=True)
    say("1 no state file yet", not (host.root / "gen/route_learner.json").exists(),
        "gen/route_learner.json is not written before a save or a stop (write interval 6 h)")
    pid_before_stop = host.props()["MainPID"]
    host.systemctl("stop")
    saved = json.loads((host.root / "gen/route_learner.json").read_text())
    names = [row[0] for row in saved["categories"].get("my_nov6list", [])]
    say("1 stop writes the state", saved["format"] == 1 and LEARN_AAAA in names,
        f"gen/route_learner.json written at the normal stop: format 1, my_nov6list {names}")
    host.systemctl("start")
    host.wait_ready("v0.2.2", 120)
    code, body = host.http("GET", "/plugins/my_nov6list/show")
    say("1 restart restores", code == 200 and LEARN_AAAA in body and dns_query(host.dns) == ANSWER,
        "after start the learned name is listed again; DNS answers")
    pid = host.props()["MainPID"]

    # --- 4 after the update ---------------------------------------------------------------------------------
    status = host.check()
    installed = json.loads((host.root / "update/installed.json").read_text())
    say("4 no update", status["current_version"] == "v0.2.2" and status["update_available"] is False
        and status.get("config_schema_applied") == 2,
        f"current={status['current_version']} available={status['update_available']} config_schema_applied=2")
    code, reply = host.http("POST", "/api/v1/update/apply", {})
    say("4 apply without force", code == 200 and reply["installed"] is False and reply["restart_required"] is False,
        f"{code} installed=false restart_required=false")
    served_before = site.config_downloads("v0.2.2")
    tree_before = host.tree()
    code, reply = host.http("POST", "/api/v1/update/apply", {"force": True})
    say("4 force accepted", code == 200 and reply["restart_required"] is True, f"{code}")
    time.sleep(3)
    host.wait_ready("v0.2.2", 120)
    wait_for("update transaction closed", 90, lambda: host.health()["update"]["state"] == "idle")
    props = host.props()
    code, body = host.http("GET", "/plugins/my_nov6list/show")
    say("4 force reinstalled", sha256(host.exe) == sha_022 and props["MainPID"] == pid and props["NRestarts"] == "0"
        and site.config_downloads("v0.2.2") == served_before and host.tree() == tree_before
        and dns_query(host.dns) == ANSWER and LEARN_AAAA in body,
        f"same program, MainPID {props['MainPID']} (unchanged), NRestarts {props['NRestarts']}; config package not downloaded again "
        f"(no extra download; {site.config_downloads('v0.2.2')} in the whole drill: refused sha, rolled-back attempt, success); all {len(tree_before)} files unchanged; learned name kept")
    upstream.close()


def prepare_namespace(name):
    run(["ip", "netns", "add", name])
    run(["ip", "-n", name, "link", "set", "lo", "up"])


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--work", required=True, help="empty directory for this drill (removed at the end)")
    parser.add_argument("--repo", required=True, help="the repository checkout holding scripts/")
    parser.add_argument("--v021-dir", required=True)
    parser.add_argument("--release-dir", required=True)
    parser.add_argument("--config-zip", required=True)
    parser.add_argument("--config-tar", required=True)
    parser.add_argument("--keep", action="store_true", help="keep the work directory")
    parser.add_argument("--log", help="also write the STEP lines to this file")
    parser.add_argument("--netns", default="rtdrill")
    args = parser.parse_args()
    if os.geteuid() != 0:
        raise SystemExit("run as root on the test VM")
    work = Path(args.work).resolve()
    if os.environ.get(NS_ENV) != args.netns:
        # Parent: own the namespace, run the drill inside it, remove everything afterwards.
        if work.exists() and any(work.iterdir()):
            raise SystemExit(f"{work} is not empty")
        base.refuse_existing_unit()
        if Path(f"/run/netns/{args.netns}").exists():
            raise SystemExit(f"network namespace {args.netns} already exists; refusing to touch it")
        work.mkdir(parents=True, exist_ok=True)
        code = 1
        try:
            prepare_namespace(args.netns)
            argv = ["ip", "netns", "exec", args.netns, sys.executable, *sys.argv]
            code = subprocess.run(argv, env=dict(os.environ, **{NS_ENV: args.netns})).returncode
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
