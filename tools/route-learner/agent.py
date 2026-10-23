"""Runs INSIDE a role's network namespace: executes a plan of steps against the running host.

    python3 agent.py <root> <corpus.json> <audit-ports.json> <plan.json> <api port> <pid>

Steps: ready | replay(tag, pass_id, [port]) | dump(tag) | get(path) | post(path) | sleep(seconds) |
faults(value) | publish | flush_caches | rss(tag) | applies(tag, count, pass_id). Each writes its result into the root.
"""
import ipaddress
import json
import pathlib
import socket
import sys
import threading
import time
import urllib.error
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "e2e-1"))
from dnswire import decode, question  # noqa: E402

TAGS = ("my_sv4list", "my_realiplist", "my_fakeiplist", "my_nov6list", "my_nov4list")
# Audit names of the question types the replays send (SOA/PTR/HTTPS: the M2-1 differential).
QTYPES = {1: "A", 6: "SOA", 12: "PTR", 28: "AAAA", 65: "HTTPS"}
CACHES = ("cache_all", "cache_all_noleak", "cache_cn", "cache_google", "cache_google_node", "cache_node", "cache_cnmihomo")


class Agent:
    def __init__(self, root, corpus, audit_ports, api_port, pid):
        self.root, self.corpus, self.audit_ports = root, corpus, set(audit_ports)
        self.api_port, self.pid = api_port, pid
        self.sockets = {}

    def api(self, path, post=False, body=b"{}"):
        request = urllib.request.Request(f"http://127.0.0.1:{self.api_port}{path}", data=body if post else None,
                                         headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.read()

    def ready(self):
        deadline = time.monotonic() + 90
        while True:
            try:
                self.api("/api/v1/audit/start", True)
                break
            except (OSError, urllib.error.URLError):
                if time.monotonic() >= deadline:
                    raise RuntimeError("host API did not become ready")
                time.sleep(.2)
        wanted = {row["port"] for row in self.corpus}
        while True:
            listening = set()
            for table in ("udp", "udp6"):
                for line in pathlib.Path("/proc/net/" + table).read_text().splitlines()[1:]:
                    listening.add(int(line.split()[1].rsplit(":", 1)[1], 16))
            if wanted <= listening:
                break
            if time.monotonic() >= deadline:
                raise RuntimeError("DNS listeners did not become ready")
            time.sleep(.1)
        self.api("/api/v1/audit/clear", True)
        for sock in self.sockets.values():
            sock.close()
        self.sockets = {}
        for source in {row["source"] for row in self.corpus}:
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind((source, 0))
            sock.settimeout(8)
            self.sockets[source] = sock

    def replay(self, tag, pass_id, port=None):
        out = (self.root / f"{tag}.jsonl").open("w")
        for index, row in enumerate(self.corpus):
            if port is not None:
                row = dict(row, port=port)
            ident = (index + pass_id * len(self.corpus)) % 65536
            sock = self.sockets[row["source"]]
            started = time.monotonic()
            sock.sendto(question(row["name"], row["type"], ident), ("127.0.0.1", row["port"]))
            try:
                result = decode(sock.recv(65535))
                if result["id"] != ident or result["questions"] != [(row["name"] + ".", row["type"], 1)]:
                    raise ValueError("uncorrelated DNS response")
            except (OSError, ValueError) as error:
                result = {"error": str(error)}
            finished = time.monotonic()
            audit = None
            if row["port"] in self.audit_ports:
                for _ in range(10):
                    try:
                        logs = json.loads(self.api("/api/v2/audit/logs?page=1&limit=1"))["logs"]
                    except (OSError, urllib.error.URLError):
                        # The management API answers 503 while a settings apply drains it.
                        time.sleep(.05)
                        continue
                    if logs:
                        entry = logs[0]
                        client = ipaddress.ip_address(entry["client_ip"])
                        client = getattr(client, "ipv4_mapped", None) or client
                        if (entry["query_name"].rstrip(".").lower() == row["name"]
                                and entry["query_type"] == QTYPES[row["type"]]
                                and str(client) == row["source"]):
                            audit = entry
                            break
                    time.sleep(.01)
            out.write(json.dumps({"pass": pass_id, "index": index, "input": row, "dns": result, "audit": audit,
                                  "audit_expected": row["port"] in self.audit_ports,
                                  "latency_ms": (finished - started) * 1000}, ensure_ascii=False) + "\n")
        out.close()

    def dump(self, tag):
        sets = {}
        for category in TAGS:
            names = set()
            try:
                text = self.api(f"/plugins/{category}/show?limit=100000000").decode()
            except (OSError, urllib.error.URLError):
                text = ""
            for line in text.splitlines():
                fields = line.split()
                if len(fields) >= 3:
                    names.add(fields[2].split("|")[0].lower())
            sets[category] = sorted(names)
        (self.root / f"{tag}.json").write_text(json.dumps(sets))

    def rss(self, tag):
        status = pathlib.Path(f"/proc/{self.pid}/status").read_text()
        rss = int(next(line.split()[1] for line in status.splitlines() if line.startswith("VmRSS:")))
        (self.root / f"{tag}.rss.json").write_text(json.dumps({"rss_kib": rss}))

    def run(self, plan):
        for step in plan:
            op = step["op"]
            if op == "ready":
                self.ready()
            elif op == "replay":
                self.replay(step["tag"], step["pass_id"], step.get("port"))
            elif op == "dump":
                self.dump(step["tag"])
            elif op == "rss":
                self.rss(step["tag"])
            elif op in ("get", "post"):
                try:
                    self.api(step["path"], op == "post", json.dumps(step.get("body", {})).encode())
                except (OSError, urllib.error.URLError):
                    pass
            elif op == "publish":
                # What the old `domain_output` does after a write: post the rule file into its provider
                # (its `domain_set_url` names the configured API port, which this harness moves).
                for stem in ("sv4", "realip", "fakeip", "nov6", "nov4"):
                    path = self.root / "gen" / f"{stem}rule.txt"
                    if not path.exists():
                        continue
                    values = [line for line in path.read_text().splitlines() if line.strip()]
                    try:
                        self.api(f"/plugins/my_{stem}rule/post", True, json.dumps({"values": values}).encode())
                    except (OSError, urllib.error.URLError):
                        pass
            elif op == "sleep":
                time.sleep(step["seconds"])
            elif op == "faults":
                path = self.root / "faults.json"
                if step["value"] is None:
                    path.unlink(missing_ok=True)
                else:
                    path.write_text(json.dumps(step["value"]))
                time.sleep(.5)
            elif op == "flush_caches":
                for cache in CACHES:
                    try:
                        self.api(f"/plugins/{cache}/flush")
                    except (OSError, urllib.error.URLError):
                        pass
            elif op == "applies":
                failures = []
                done = threading.Event()

                def apply():
                    for number in range(step["count"]):
                        proxy = "127.0.0.1:17892" if number % 2 == 0 else "127.0.0.1:17891"
                        try:
                            self.api("/api/v1/overrides", True,
                                     json.dumps({"socks5": proxy, "ecs": "", "replacements": []}).encode())
                        except Exception as error:  # noqa: BLE001
                            failures.append(repr(error))
                        time.sleep(1.5)
                    done.set()

                thread = threading.Thread(target=apply)
                thread.start()
                self.replay(step["tag"], step["pass_id"])
                thread.join()
                (self.root / f"{step['tag']}.applies.json").write_text(json.dumps({"failures": failures}))
            else:
                raise ValueError("unknown step " + op)


if __name__ == "__main__":
    root, corpus, audit, plan, api_port, pid = sys.argv[1:7]
    agent = Agent(pathlib.Path(root), json.loads(pathlib.Path(corpus).read_text()),
                  json.loads(pathlib.Path(audit).read_text()), int(api_port), int(pid))
    agent.run(json.loads(pathlib.Path(plan).read_text()))
