#!/usr/bin/env python3
"""VM-only drill of the online-update path after the rtdns rename (task 10-22-rtdns-rename-release).

Runs a systemd unit named `rtdns` (Restart=always, RestartSec=120, program at
<work>/usr/local/bin/rtdns, configuration root <work>/cus/rtdns) against a fake GitHub
release API on 127.0.0.1 (MOSDNS_UPDATE_API_URL) and a fake UDP upstream on 127.0.0.1.
Release artifacts are built with scripts/build-rust-release.sh from prebuilt binaries
that report v0.2.0 / v0.2.1. Never touches the public network; only RFC 5737 answers.

    drill.py --work DIR --repo DIR --v020 BIN --v021 BIN

Needs root, systemd, jq and tar. Removes the unit, the processes and (unless --keep)
the work directory when it ends, whatever the outcome.
"""
import argparse
import hashlib
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
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

UNIT = "rtdns"
UNIT_FILE = Path(f"/etc/systemd/system/{UNIT}.service")
UNIT_OWNED = False   # true once this run has written the unit file: only then is it cleaned up
ANSWER = "192.0.2.1"
ASSET = "rtdns-linux-amd64.tar.gz"
MANIFEST = "rtdns-update-manifest.json"
RESULTS = []


def say(step, ok, detail=""):
    line = f"STEP {step}: {'OK' if ok else 'FAIL'} {detail}".rstrip()
    print(line, flush=True)
    RESULTS.append(line)
    if not ok:
        raise SystemExit(f"drill failed at step {step}: {detail}")


def sha256(path_or_bytes):
    data = path_or_bytes if isinstance(path_or_bytes, bytes) else Path(path_or_bytes).read_bytes()
    return hashlib.sha256(data).hexdigest()


def run(argv, **kwargs):
    return subprocess.run(list(map(str, argv)), check=True, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, timeout=600, **kwargs).stdout.decode()


def wait_for(what, seconds, condition):
    end = time.monotonic() + seconds
    last = None
    while time.monotonic() < end:
        try:
            last = condition()
            if last:
                return last
        except (OSError, ValueError, KeyError, urllib.error.URLError, subprocess.SubprocessError):
            pass
        time.sleep(0.1)
    raise SystemExit(f"timed out waiting for {what} (last={last!r})")


def free_port(kind=socket.SOCK_STREAM):
    with socket.socket(socket.AF_INET, kind) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


# ----- fake upstream ---------------------------------------------------------------------

def start_upstream(port):
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
            answer = b""
            if qtype == 1:
                answer = b"\xc0\x0c" + struct.pack(">HHIH", 1, 1, 60, 4) + socket.inet_aton(ANSWER)
            header = data[:2] + struct.pack(">HHHHH", 0x8180, 1, 1 if answer else 0, 0, 0)
            sock.sendto(header + question + answer, peer)

    threading.Thread(target=serve, daemon=True).start()
    return sock


def dns_query(port, name="drill.example.com", timeout=0.5):
    labels = b"".join(bytes([len(part)]) + part.encode() for part in name.split("."))
    query = struct.pack(">HHHHHH", 0x1234, 0x0100, 1, 0, 0, 0) + labels + b"\0" + struct.pack(">HH", 1, 1)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.settimeout(timeout)
        sock.sendto(query, ("127.0.0.1", port))
        reply, _ = sock.recvfrom(2048)
    if struct.unpack(">H", reply[6:8])[0] == 0:
        return None
    position = reply.index(b"\0", 12) + 5   # end of the question
    position += 2 if reply[position] & 0xC0 == 0xC0 else reply.index(b"\0", position) + 1 - position
    return socket.inet_ntoa(reply[position + 10:position + 14])


class Prober:
    """A DNS query every 5 ms with a 200 ms timeout; reports failures and the longest silence."""

    def __init__(self, port):
        self.port, self.stop = port, threading.Event()
        self.total = self.failed = 0
        self.longest = 0.0
        self.thread = threading.Thread(target=self.loop, daemon=True)

    def loop(self):
        last_ok = time.monotonic()
        while not self.stop.is_set():
            self.total += 1
            try:
                ok = dns_query(self.port, timeout=0.2) == ANSWER
            except OSError:
                ok = False
            now = time.monotonic()
            if ok:
                self.longest = max(self.longest, now - last_ok)
                last_ok = now
            else:
                self.failed += 1
            time.sleep(0.005)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_):
        self.stop.set()
        self.thread.join()

    def summary(self):
        return f"{self.total} queries, {self.failed} failed, longest silence {self.longest:.3f}s"


# ----- fake GitHub -----------------------------------------------------------------------

class Site:
    def __init__(self):
        self.releases = []   # [{"tag", "draft", "prerelease", "assets": [names]}]
        self.files = {}      # "/dl/<tag>/<name>" -> bytes
        self.listing_requests = 0
        self.port = free_port()
        site = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                path = self.path.split("?")[0]
                if path == "/releases":
                    site.listing_requests += 1
                    body = json.dumps(site.listing()).encode()
                elif path in site.files:
                    body = site.files[path]
                else:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = ThreadingHTTPServer(("127.0.0.1", self.port), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    @property
    def api_url(self):
        return f"http://127.0.0.1:{self.port}/releases?per_page=100"

    def listing(self):
        out = []
        for release in self.releases:
            tag = release["tag"]
            assets = []
            for name in release["assets"]:
                data = self.files.get(f"/dl/{tag}/{name}", b"")
                assets.append({"name": name, "updated_at": "2026-10-22T00:00:00Z",
                               "browser_download_url": f"http://127.0.0.1:{self.port}/dl/{tag}/{name}",
                               "digest": "sha256:" + sha256(data)})
            out.append({"tag_name": tag, "draft": release["draft"], "prerelease": release["prerelease"],
                        "published_at": "2026-10-22T00:00:00Z", "assets": assets})
        return out

    def publish(self, tag, directory, draft=False, prerelease=False, manifest=None):
        names = [ASSET, MANIFEST]
        for name in names:
            data = Path(directory, name).read_bytes()
            if name == MANIFEST and manifest is not None:
                data = manifest
            self.files[f"/dl/{tag}/{name}"] = data
        self.add(tag, draft, prerelease, names)

    def add(self, tag, draft=False, prerelease=False, assets=()):
        self.releases = [r for r in self.releases if r["tag"] != tag]
        self.releases.append({"tag": tag, "draft": draft, "prerelease": prerelease, "assets": list(assets)})


# ----- the host under test ---------------------------------------------------------------

class Host:
    def __init__(self, work, site):
        self.work, self.site = Path(work), site
        self.exe = self.work / "usr/local/bin/rtdns"
        self.root = self.work / "cus/rtdns"
        self.api, self.dns, self.upstream = free_port(), free_port(socket.SOCK_DGRAM), free_port(socket.SOCK_DGRAM)

    def config_text(self):
        return (f"log: {{level: info}}\napi: {{http: '127.0.0.1:{self.api}'}}\nplugins:\n"
                f"  - tag: cache_main\n    type: cache\n    args: {{}}\n"
                f"  - tag: upstream\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:{self.upstream}'}}]}}\n"
                f"  - tag: main\n    type: sequence\n    args: [{{exec: $cache_main}}, {{exec: $upstream}}]\n"
                f"  - tag: dns\n    type: udp_server\n    args: {{listen: '127.0.0.1:{self.dns}', entry: main}}\n")

    def install_unit(self):
        global UNIT_OWNED
        refuse_existing_unit()
        self.exe.parent.mkdir(parents=True, exist_ok=True)
        self.root.mkdir(parents=True, exist_ok=True)
        (self.root / "config_custom.yaml").write_text(self.config_text())
        # What the production root has since the first installation (cutover): the package
        # state at the schema the program requires, so a binary-only update needs no package.
        (self.root / "webinfo").mkdir()
        (self.root / "webinfo/rust_config_update_state.json").write_text(json.dumps(
            {"format": 1, "applied_schema": 1, "required_schema": 1, "status": "success",
             "package_id": "rust-config-schema-1", "binary_version": "v0.2.0"}))
        UNIT_FILE.write_text(
            "[Unit]\nDescription=rtdns rename drill (temporary)\nAfter=network.target\n\n"
            "[Service]\nType=simple\n"
            f"Environment=MOSDNS_UPDATE_API_URL={self.site.api_url}\n"
            "Environment=MOSDNS_UPDATE_HEALTH_TIMEOUT_SECS=20\n"
            f"ExecStart={self.exe} start -d {self.root} -c {self.root}/config_custom.yaml\n"
            "Restart=always\nRestartSec=120\n\n[Install]\nWantedBy=multi-user.target\n")
        UNIT_OWNED = True
        run(["systemctl", "daemon-reload"])

    def systemctl(self, *args):
        return run(["systemctl", *args, UNIT]).strip()

    def props(self):
        out = run(["systemctl", "show", UNIT, "-p", "MainPID", "-p", "NRestarts", "-p", "ActiveState"])
        return dict(line.split("=", 1) for line in out.splitlines())

    def http(self, method, path, body=None, timeout=15):
        request = urllib.request.Request(f"http://127.0.0.1:{self.api}{path}", method=method,
                                         data=None if body is None else json.dumps(body).encode())
        try:
            with urllib.request.urlopen(request, timeout=timeout) as reply:
                return reply.status, json.loads(reply.read() or b"null")
        except urllib.error.HTTPError as error:
            raw = error.read()
            try:
                return error.code, json.loads(raw)
            except ValueError:
                return error.code, raw.decode(errors="replace")

    def health(self):
        status, body = self.http("GET", "/api/v1/system/health", timeout=3)
        return body if status == 200 else None

    def wait_ready(self, version, seconds=60):
        return wait_for(f"health ready as {version}", seconds,
                        lambda: (lambda h: h if h and h.get("ready") and h.get("version") == version else None)(self.health()))

    def check(self):
        status, body = self.http("POST", "/api/v1/update/check", {})
        assert status == 200, (status, body)
        return body

    def update_dir_names(self):
        directory = self.root / "update"
        return sorted(p.name for p in directory.iterdir()) if directory.exists() else []


def build_release(repo, binary, tag, out):
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, PREBUILT=str(binary))
    run([Path(repo) / "scripts/build-rust-release.sh", tag, out], env=env)
    return out


def drill(args):
    work = Path(args.work)
    repo = Path(args.repo)
    site = Site()
    host = Host(work, site)
    upstream = start_upstream(host.upstream)
    releases = work / "releases"
    # Release artifacts through the release script, from prebuilt binaries.
    v020 = build_release(repo, args.v020, "v0.2.0", releases / "v0.2.0")
    v021 = build_release(repo, args.v021, "v0.2.1", releases / "v0.2.1")
    manifest = json.loads((v021 / MANIFEST).read_text())
    say("0 build", manifest["version"] == "v0.2.1" and list(manifest["artifacts"]) == [ASSET],
        f"artifacts {sorted(p.name for p in v021.iterdir())}; v0.2.1 sha256 {manifest['artifacts'][ASSET]['sha256'][:12]}")
    for tag in ("rust-v0.2.1", "v0.2", "V0.2.1"):
        refused = subprocess.run([repo / "scripts/build-rust-release.sh", tag, work / "refused"],
                                 env=dict(os.environ, PREBUILT=str(args.v021)),
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        say(f"0 reject-tag {tag}", refused.returncode != 0 and not (work / "refused").exists(), refused.stdout.decode().strip())

    host.install_unit()
    program_v020 = work / "unpacked-v0.2.0"
    program_v020.mkdir(parents=True)
    run(["tar", "-xzf", v020 / ASSET, "-C", program_v020])
    sha_020 = sha256(program_v020 / "rtdns")
    sha_021 = sha256(Path(args.v021))
    config_before = (host.root / "config_custom.yaml").read_bytes()

    # --- 1. the user replaces the program by hand: stop, copy, start --------------------
    site.releases = []
    shutil.copy2(program_v020 / "rtdns", host.exe)
    host.systemctl("start")
    health = host.wait_ready("v0.2.0")
    say("1 manual replace: health", health["version"] == "v0.2.0", f"version={health['version']}")
    say("1 manual replace: dns", dns_query(host.dns) == ANSWER, f"answer {ANSWER}")
    site.publish("v0.2.0", v020)
    status = host.check()
    say("1 manual replace: status", status["current_version"] == "v0.2.0" and status["latest_version"] == "v0.2.0"
        and status["update_available"] is False and not status.get("current_signature")
        and not status.get("rollback_performed") and not status.get("pending_restart"),
        f"current={status['current_version']} latest={status['latest_version']} available={status['update_available']} "
        f"current_signature={status.get('current_signature', '')!r}")
    say("1 manual replace: health.update", host.health()["update"]["state"] == "idle", "idle")
    pid_before = host.props()["MainPID"]

    # --- 2/4. the check sees v0.2.1 and ignores every other tag family ----------------------
    site.publish("v0.2.1", v021)
    site.publish("rust-v0.2.5", v021)
    site.publish("lite-v0.2.5", v021)
    site.publish("openwrt-v0.2.5", v021)
    site.publish("V0.2.6", v021)
    site.publish("v0.2.7", v021, draft=True)
    site.publish("v0.2.8", v021, prerelease=True)
    status = host.check()
    say("2 check", status["latest_version"] == "v0.2.1" and status["update_available"] is True
        and status["release_url"] == "https://github.com/jasonxtt/rtdns/releases/tag/v0.2.1"
        and status["asset_name"] == ASSET and status["download_url"].endswith(f"/dl/v0.2.1/{ASSET}"),
        f"latest={status['latest_version']} release_url={status['release_url']}")
    say("4 other tag families ignored", True,
        "rust-v0.2.5, lite-v0.2.5, openwrt-v0.2.5, V0.2.6, draft v0.2.7 and prerelease v0.2.8 were listed; latest stayed v0.2.1")

    with Prober(host.dns) as probe:
        status_code, reply = host.http("POST", "/api/v1/update/apply", {})
        say("2 apply: accepted", status_code == 200 and isinstance(reply, dict) and reply.get("restart_required") is True,
            f"{status_code} {reply if status_code != 200 else reply.get('notes', '')}")
        host.wait_ready("v0.2.1", 90)
        wait_for("update transaction closed", 60, lambda: host.health()["update"]["state"] == "idle")
        time.sleep(1)
    props = host.props()
    installed = json.loads((host.root / "update/installed.json").read_text())
    say("2 apply: upgraded", sha256(host.exe) == sha_021 and props["MainPID"] == pid_before and props["NRestarts"] == "0"
        and installed["version"] == "v0.2.1" and dns_query(host.dns) == ANSWER,
        f"program sha256 == v0.2.1 build; MainPID {props['MainPID']} (unchanged), NRestarts {props['NRestarts']}, "
        f"installed.json version {installed['version']}; DNS during upgrade: {probe.summary()}")
    say("2 apply: config untouched", (host.root / "config_custom.yaml").read_bytes() == config_before, "config_custom.yaml identical")

    # --- 3. nothing newer; force reinstalls ---------------------------------------------------
    status = host.check()
    say("3 no update", status["current_version"] == "v0.2.1" and status["update_available"] is False
        and status.get("current_signature", "") == installed["asset_signature"],
        f"available={status['update_available']} current_signature matches installed.json")
    code, reply = host.http("POST", "/api/v1/update/apply", {})
    say("3 apply without force", code == 200 and reply["installed"] is False and reply["restart_required"] is False, f"{code} installed=false restart_required=false")
    code, reply = host.http("POST", "/api/v1/update/apply", {"force": True})
    say("3 force: accepted", code == 200 and reply["restart_required"] is True, f"{code}")
    # The guard swaps the program twice (through the old copy, then the confirmed program).
    time.sleep(3)
    host.wait_ready("v0.2.1", 90)
    wait_for("update transaction closed", 60, lambda: host.health()["update"]["state"] == "idle")
    props = host.props()
    say("3 force: reinstalled", sha256(host.exe) == sha_021 and props["MainPID"] == pid_before and props["NRestarts"] == "0"
        and dns_query(host.dns) == ANSWER, f"same program, MainPID {props['MainPID']}, NRestarts {props['NRestarts']}")

    # --- 5a. a hash that does not match is refused and nothing changes -----------------------
    bad = json.loads((v021 / MANIFEST).read_text())
    bad["version"] = "v0.2.2"
    bad["artifacts"][ASSET]["sha256"] = "0" * 64
    site.publish("v0.2.2", v021, manifest=json.dumps(bad).encode())
    status = host.check()
    assert status["latest_version"] == "v0.2.2", status
    names_before = host.update_dir_names()
    code, reply = host.http("POST", "/api/v1/update/apply", {})
    time.sleep(2)
    props = host.props()
    say("5a sha256 mismatch refused", code == 502 and "SHA-256" in str(reply) and sha256(host.exe) == sha_021
        and (host.root / "config_custom.yaml").read_bytes() == config_before and host.update_dir_names() == names_before
        and host.health()["version"] == "v0.2.1" and props["MainPID"] == pid_before,
        f"{code} {reply}; program, config and update/ {names_before} unchanged; MainPID {props['MainPID']}")

    # --- 5b. a candidate that is not healthy is rolled back automatically -------------------
    broken = work / "broken"
    broken.mkdir()
    script = broken / "rtdns"
    script.write_text("#!/bin/sh\ncase \"$1\" in\n  version) echo v0.2.3;;\n  check) exit 0;;\n  start) exit 3;;\nesac\n")
    script.chmod(0o755)
    build_release(repo, script, "v0.2.3", broken / "release")
    site.publish("v0.2.3", broken / "release")
    status = host.check()
    assert status["latest_version"] == "v0.2.3", status
    code, reply = host.http("POST", "/api/v1/update/apply", {})
    say("5b apply: accepted", code == 200, f"{code}")
    health = wait_for("rollback reported", 90, lambda: (lambda h: h if h and h.get("update", {}).get("state") == "rolled_back"
                                                      and h.get("version") == "v0.2.1" and h.get("ready") else None)(host.health()))
    props = host.props()
    status = host.check()
    say("5b rolled back", sha256(host.exe) == sha_021 and (host.root / "config_custom.yaml").read_bytes() == config_before
        and props["MainPID"] == pid_before and props["NRestarts"] == "0" and status.get("rollback_performed") is True
        and dns_query(host.dns) == ANSWER,
        f"program is v0.2.1 again, MainPID {props['MainPID']}, NRestarts {props['NRestarts']}, "
        f"reason {health['update'].get('last_error')!r}")
    upstream.close()


def refuse_existing_unit():
    """Exit before touching anything when a unit of that name exists anywhere systemd looks."""
    found = [str(p) for p in (Path(d, f"{UNIT}.service") for d in (
        "/etc/systemd/system", "/run/systemd/system", "/usr/lib/systemd/system", "/lib/systemd/system"))
        if p.exists() or p.is_symlink()]
    known = subprocess.run(["systemctl", "cat", f"{UNIT}.service"], stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL).returncode == 0
    if found or known:
        raise SystemExit(f"a unit named {UNIT}.service already exists ({found or 'known to systemd'}); refusing to touch it")


def cleanup(work, keep):
    if UNIT_OWNED:
        quiet = {"stdout": subprocess.DEVNULL, "stderr": subprocess.DEVNULL}
        subprocess.run(["systemctl", "stop", UNIT], **quiet)
        subprocess.run(["systemctl", "disable", UNIT], **quiet)
        UNIT_FILE.unlink(missing_ok=True)
        subprocess.run(["systemctl", "daemon-reload"], **quiet)
        subprocess.run(["systemctl", "reset-failed", UNIT], **quiet)
    if not keep:
        shutil.rmtree(work, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--work", required=True, help="empty directory for this drill (removed at the end)")
    parser.add_argument("--repo", required=True, help="the repository checkout holding scripts/")
    parser.add_argument("--v020", required=True, help="a prebuilt rtdns reporting v0.2.0")
    parser.add_argument("--v021", required=True, help="a prebuilt rtdns reporting v0.2.1")
    parser.add_argument("--keep", action="store_true", help="keep the work directory")
    parser.add_argument("--log", help="also write the STEP lines to this file")
    args = parser.parse_args()
    if os.geteuid() != 0:
        raise SystemExit("run as root on the test VM")
    refuse_existing_unit()
    work = Path(args.work).resolve()
    if work.exists() and any(work.iterdir()):
        raise SystemExit(f"{work} is not empty")
    work.mkdir(parents=True, exist_ok=True)
    args.work = str(work)
    for name in ("v020", "v021"):
        version = subprocess.run([getattr(args, name), "version"], stdout=subprocess.PIPE).stdout.decode().strip()
        want = "v0.2.0" if name == "v020" else "v0.2.1"
        if version != want:
            raise SystemExit(f"--{name} reports {version!r}, not {want}")
    try:
        drill(args)
        print("DRILL PASSED", flush=True)
    finally:
        try:
            if args.log:
                Path(args.log).write_text("\n".join(RESULTS) + "\n")
        finally:
            cleanup(work, args.keep)


if __name__ == "__main__":
    main()
