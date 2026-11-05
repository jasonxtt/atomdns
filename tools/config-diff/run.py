#!/usr/bin/env python3
"""Routing differential between two AtomDNS default-config trees.

Runs the same native binary once per (config tree x flag combination) on loopback with
mock upstreams, replays one family of controlled names (a name encodes which mark list it
belongs to and what each upstream answers) through the main entry and the requery entry,
and prints every case whose observable behaviour differs between the two trees:

  * the client response (rcode and answer values, TTL ignored)
  * which mock upstreams were asked, in order of first contact
  * the audit record: final_sequence, final_upstream, domain_set, effective_tag

Nothing here touches the network: all upstream groups are rewritten to the mocks, the
sd_set_light/si_set rule sets are replaced by small synthetic files. Run it on the Linux
test host (it starts the native binary); it never builds anything.

  python3 run.py --old <config_rust tree> --new <config_rust tree> --binary <atomdns> \
      --work <fresh dir> [--combos safe,cnfake,...] [--json out.json]
"""
import argparse
import ipaddress
import itertools
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
import urllib.request

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "e2e-1"))
import dnswire  # noqa: E402

DNS, API, RQ = 25353, 29099, 25354
MOCK_PORTS = {"domestic": 25401, "foreign": 25402, "ecs": 25403, "fake": 25404, "cnfake": 25405}
CN_V4, FO_V4, DIP_V4 = "114.114.114.114", "8.8.8.8", "17.1.1.1"
CN_V6, FO_V6, ULA_V6 = "240e:1::1", "2001:4860::8888", "fd00::1"

# class -> (suffix domain, rule file or inline list that holds it)
LISTS = {
    "unl": None,
    "bl": "blocklist", "dd": "ddnslist", "gl": "greylist", "wl": "whitelist",
    "ri": "realiplist", "cf": "cnfakeipfilter",
    "cc": "h_cuscn", "cn": "h_cusnocn", "gn": "h_geosite_no_cn", "gc": "h_geosite_cn",
    # several lists at once: the order of the flow decides
    "wg": ("whitelist", "greylist"), "wc": ("whitelist", "h_geosite_cn"),
    "gc2": ("greylist", "h_cuscn"), "cg": ("h_cuscn", "h_geosite_no_cn"),
    "cng": ("h_geosite_cn", "h_geosite_no_cn"), "rc": ("realiplist", "cnfakeipfilter"),
}
FLAGS = {  # the flag combinations to run: name -> flags.json
    "compat": {},
    "safe": {"core_mode": "safe"},
    "compat+cnfake": {"cn_fakeip": True},
    "safe+cnfake": {"core_mode": "safe", "cn_fakeip": True},
    "compat+realip": {"dns_mode": "realip"},
    "safe+realip": {"core_mode": "safe", "dns_mode": "realip"},
    "compat+cnfake+realip": {"cn_fakeip": True, "dns_mode": "realip"},
    "safe+cnfake+realip": {"core_mode": "safe", "cn_fakeip": True, "dns_mode": "realip"},
    "safe+ddnsforeign": {"core_mode": "safe", "ddns_foreign": True},
    "compat+ddnsforeign": {"ddns_foreign": True},
    # caches off: the second query of a name reaches the remembered-route leaves instead of a cache
    "compat+nocache": {"full_cache": False, "lazy_cache": False},
    "safe+nocache": {"core_mode": "safe", "full_cache": False, "lazy_cache": False},
    "compat+cnfake+nocache": {"cn_fakeip": True, "full_cache": False, "lazy_cache": False},
    "safe+cnfake+nocache": {"core_mode": "safe", "cn_fakeip": True, "full_cache": False, "lazy_cache": False},
}


# ---- mock upstreams -------------------------------------------------------------------

def rr(qtype, value):
    raw = ipaddress.ip_address(value).packed
    return b"\xc0\x0c" + struct.pack("!HHIH", qtype, 1, 60, len(raw)) + raw


def reply(query, rcode, answers):
    _, end = dnswire.name(query, 12)
    end += 4
    flags = 0x8180 | rcode
    header = query[:2] + struct.pack("!H", flags) + struct.pack("!4H", 1, len(answers), 0, 0)
    return header + query[12:end] + b"".join(answers)


def code_answer(code, qtype):
    """(rcode, [records]) for an answer code; None = no reply at all.

    `a_b` answers A with code `a` and AAAA with code `b` (e.g. `cn_fo`: a CN IPv4 but a
    foreign IPv6), to model dual-stack names whose families disagree."""
    if "_" in code:
        first, second = code.split("_", 1)
        code = first if qtype == 1 else second
    if code == "fail":
        return 2, []
    if code == "nx":
        return 3, []
    v4 = {"cn": CN_V4, "fo": FO_V4, "dip": DIP_V4}.get(code)
    v6 = {"cn": CN_V6, "fo": FO_V6, "ula": ULA_V6}.get(code)
    if qtype == 1 and v4:
        return 0, [rr(1, v4)]
    if qtype == 28 and v6:
        return 0, [rr(28, v6)]
    return 0, []


class Mocks:
    """`delays` maps a qtype to seconds the real-IP mocks wait before answering it;
    `fake_v6=False` makes the FakeIP mocks answer AAAA with an empty NOERROR, like a
    FakeIP server that hands out only IPv4."""

    def __init__(self, delays=None, fake_v6=True):
        self.delays = delays or {}
        self.fake_v6 = fake_v6
        self.hits = []
        self.lock = threading.Lock()
        self.socks = {}
        for kind, port in MOCK_PORTS.items():
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            sock.bind(("127.0.0.1", port))
            self.socks[kind] = sock
            threading.Thread(target=self.serve, args=(kind, sock), daemon=True).start()

    def serve(self, kind, sock):
        while True:
            try:
                data, peer = sock.recvfrom(4096)
            except OSError:
                return
            try:
                qname, end = dnswire.name(data, 12)
                qtype = struct.unpack_from("!H", data, end)[0]
            except Exception:
                continue
            with self.lock:
                self.hits.append((kind, qtype))
            labels = qname.split(".")
            if kind in ("fake", "cnfake"):
                if qtype == 1:
                    answers = [rr(1, "28.0.0.7" if kind == "fake" else "30.0.0.7")]
                elif qtype == 28 and self.fake_v6:
                    answers = [rr(28, "2001:2::7" if kind == "fake" else "2400::7")]
                else:
                    answers = []
                sock.sendto(reply(data, 0, answers), peer)
                continue
            code = labels[0] if kind == "domestic" else labels[1]
            rcode, answers = code_answer(code, qtype)
            delay = self.delays.get(qtype, 0)
            if delay:
                threading.Timer(delay, sock.sendto, args=(reply(data, rcode, answers), peer)).start()
            else:
                sock.sendto(reply(data, rcode, answers), peer)

    def take(self):
        with self.lock:
            hits, self.hits = self.hits, []
        order = []
        for kind, qtype in hits:
            if (kind, qtype) not in order:
                order.append((kind, qtype))
        return [f"{kind}:{qtype}" for kind, qtype in order]


# ---- staging a config tree -------------------------------------------------------------

def patch(path, pairs, regex=False):
    text = open(path, encoding="utf-8").read()
    for old, new in pairs:
        if regex:
            text, count = re.subn(old, new, text, flags=re.S)
        else:
            count = text.count(old)
            text = text.replace(old, new)
        if not count:
            raise SystemExit(f"{path}: pattern not found: {old!r}")
    open(path, "w", encoding="utf-8").write(text)


def stage(source, target, flags):
    shutil.copytree(source, target)
    sub = os.path.join(target, "sub_config")
    udp = lambda kind: f'"udp://127.0.0.1:{MOCK_PORTS[kind]}"'  # noqa: E731
    patch(os.path.join(sub, "forward_local.yaml"), [
        (r'addr: "udp://223\.5\.5\.5"', f"addr: {udp('domestic')}"),
        (r'addr: "https://dns\.alidns\.com/dns-query"\s*\n\s*dial_addr: "223\.5\.5\.5"', f"addr: {udp('domestic')}")], regex=True)
    patch(os.path.join(sub, "forward_nocn.yaml"), [
        (r'addr: "https://dns\.google/dns-query"\s*\n\s*dial_addr: "8\.8\.8\.8"', f"addr: {udp('foreign')}"),
        (r'addr: "https://1\.1\.1\.1/dns-query"', f"addr: {udp('foreign')}")], regex=True)
    patch(os.path.join(sub, "forward_nocn_ecs.yaml"), [
        (r'addr: "https://dns\.google/dns-query"\s*\n\s*dial_addr: "8\.8\.8\.8"', f"addr: {udp('ecs')}")], regex=True)
    patch(os.path.join(sub, "forward_1.yaml"), [
        ('addr: "udp://127.0.0.1:6666"', f"addr: {udp('fake')}"),
        ('addr: "udp://127.0.0.1:1053"', f"addr: {udp('cnfake')}")])
    # synthetic rule sets instead of the real .srs files
    rules = os.path.join(sub, "rule_set.yaml")
    for tag in ("cuscn", "cusnocn", "geosite_cn", "geosite_no_cn"):
        patch(rules, [(rf"- tag: {tag}\s*\n\s*type: sd_set_light\s*\n\s*args:.*?local_config: \"[^\"]+\"",
                       f"- tag: {tag}\n    type: domain_set_light\n    args:\n      files: [\"rule/h_{tag}.txt\"]")], regex=True)
    patch(rules, [(r"- tag: geoip_cn\s*\n\s*type: si_set\s*\n\s*args:.*?local_config: \"[^\"]+\"",
                   "- tag: geoip_cn\n    type: ip_set\n    args:\n      files: [\"rule/h_geoip_cn.txt\"]")], regex=True)
    main = os.path.join(target, "config_custom.yaml")
    patch(main, [('listen: ":53"', f'listen: "127.0.0.1:{DNS}"'), ('"0.0.0.0:9099"', f'"127.0.0.1:{API}"')])
    with open(main, "a", encoding="utf-8") as handle:
        handle.write(f"\n  - tag: udp_rq\n    type: udp_server\n    args: {{entry: sequence_requery, listen: \"127.0.0.1:{RQ}\", enable_audit: true}}\n")
    # lists
    files = {}
    for cls, spec in LISTS.items():
        if spec is None:
            continue
        for name in (spec if isinstance(spec, tuple) else (spec,)):
            files.setdefault(name, []).append(f"domain:{cls}.test")
    for name in ("h_cuscn", "h_cusnocn", "h_geosite_cn", "h_geosite_no_cn"):
        files.setdefault(name, [])
    rule_dir = os.path.join(target, "rule")
    for name, lines in files.items():
        with open(os.path.join(rule_dir, f"{name}.txt"), "w", encoding="utf-8") as handle:
            handle.write("".join(line + "\n" for line in lines))  # a bare name would mean full:
    with open(os.path.join(rule_dir, "h_geoip_cn.txt"), "w") as handle:
        handle.write("114.0.0.0/8\n240e::/16\n")
    for stale in ("gen/route_learner.json",):
        path = os.path.join(target, stale)
        if os.path.exists(path):
            os.remove(path)
    os.makedirs(os.path.join(target, "webinfo"), exist_ok=True)
    with open(os.path.join(target, "webinfo", "settings.json"), "w") as handle:
        json.dump({"version": 1, "flags": flags}, handle)
    return target


# ---- running ---------------------------------------------------------------------------

def http(method, path, body=None):
    request = urllib.request.Request(f"http://127.0.0.1:{API}{path}", method=method, data=body)
    with urllib.request.urlopen(request, timeout=10) as response:
        return response.read()


def start(binary, directory):
    log = open(os.path.join(directory, "run.log"), "w")
    process = subprocess.Popen([binary, "start", "-d", directory, "-c", "config_custom.yaml"], stdout=log, stderr=log)
    for _ in range(100):
        try:
            http("GET", "/api/v1/audit/status")
            break
        except Exception:
            if process.poll() is not None:
                raise SystemExit(f"host exited, see {directory}/run.log")
            time.sleep(0.2)
    else:
        raise SystemExit("host did not come up")
    try:
        http("POST", "/api/v1/audit/start")
    except Exception:
        pass
    return process


def ask(port, name, qtype):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.settimeout(8)
    sock.sendto(dnswire.question(name, qtype, 0x1234), ("127.0.0.1", port))
    try:
        data = sock.recv(4096)
    except socket.timeout:
        return {"rcode": "timeout", "answers": []}
    finally:
        sock.close()
    message = dnswire.decode(data)
    return {"rcode": message["rcode"], "answers": [(a[1], a[3]) for a in message["answers"]]}


def last_audit():
    """Newest audit record's routing fields."""
    page = json.loads(http("GET", "/api/v2/audit/logs?page=1&limit=1"))
    rows = page.get("logs") or page.get("records") or page.get("items") or []
    if not rows:
        return {}
    row = rows[0]
    record = {key: row.get(key) for key in ("final_sequence", "final_upstream", "domain_set", "effective_tag", "cache_status")}
    # the two entries of a group race when both point at the same mock: name the group
    groups = {"阿里UDP": "domestic", "阿里DOH": "domestic", "google": "foreign", "cloudflare": "foreign"}
    record["final_upstream"] = groups.get(record["final_upstream"], record["final_upstream"])
    return record


def cases():
    d_codes = ["cn", "fo", "none", "fail"]
    f_codes = ["cn", "fo", "none", "fail", "dip", "ula"]
    for cls in LISTS:
        if cls == "unl":
            pairs = list(itertools.product(d_codes, f_codes))
        else:
            pairs = list(itertools.product(["cn", "none"], ["fo", "none"]))
        for d, f in pairs:
            yield cls, d, f


def run_tree(label, source, binary, work, combos, mocks, qtypes, only):
    results = {}
    for combo in combos:
        directory = stage(source, os.path.join(work, f"{label}-{combo}"), FLAGS[combo])
        process = start(binary, directory)
        try:
            serial = 0
            for cls, d, f in cases():
                if only and cls not in only:
                    continue
                for entry, port in (("m", DNS), ("r", RQ)):
                    for qtype in qtypes:
                        serial += 1
                        name = f"{d}.{f}.{cls}-{entry}{qtype}-{serial}.{cls}.test"
                        for repeat in (1, 2):
                            mocks.take()
                            response = ask(port, name, qtype)
                            time.sleep(0.02)
                            record = {"response": response, "asked": mocks.take(), "audit": last_audit()}
                            key = f"{combo}|{entry}|{qtype}|{cls}|{d}|{f}|{repeat}"
                            results[key] = record
        finally:
            process.terminate()
            process.wait(timeout=20)
    return results


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--old", required=True)
    parser.add_argument("--new", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--work", required=True)
    parser.add_argument("--combos", default=",".join(FLAGS))
    parser.add_argument("--qtypes", default="1,28")
    parser.add_argument("--only", default="")
    parser.add_argument("--json")
    args = parser.parse_args()
    os.makedirs(args.work)
    combos = args.combos.split(",")
    qtypes = [int(x) for x in args.qtypes.split(",")]
    only = set(x for x in args.only.split(",") if x)
    mocks = Mocks()
    old = run_tree("old", args.old, args.binary, args.work, combos, mocks, qtypes, only)
    new = run_tree("new", args.new, args.binary, args.work, combos, mocks, qtypes, only)
    different = {key: (old[key], new[key]) for key in old if old[key] != new.get(key)}
    print(f"cases: {len(old)}  differing: {len(different)}")
    by_combo = {}
    for key in different:
        by_combo.setdefault(key.split("|")[0], []).append(key)
    for combo, keys in by_combo.items():
        print(f"\n== {combo}: {len(keys)} differ")
        for key in keys[:12]:
            a, b = different[key]
            print(f"  {key}")
            for field in ("response", "asked", "audit"):
                if a[field] != b[field]:
                    print(f"    {field}: old={a[field]}\n    {' ' * len(field)}  new={b[field]}")
    if args.json:
        with open(args.json, "w") as handle:
            json.dump({"old": old, "new": new}, handle, indent=1, default=str)


if __name__ == "__main__":
    main()
