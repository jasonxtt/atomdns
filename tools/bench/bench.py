#!/usr/bin/env python3
"""Fixed DNS benchmark driver (runs on the workstation, drives two hosts over ssh).

  bench.py --load-host <ssh alias> --sut-host <ssh alias> --server <ip:port> \
           --process <process name> --label <name> --out <local results dir>

The load host runs dnsperf (unpacked to ~/bench/dnsperf.sh) against <server>; the
fake upstream (upstream.sh) and the query files (gen_corpus.py, ~/bench/corpus) must
already be there. The server under test (SUT) must already be running on the SUT host
as <process>; it is only measured, never started or stopped by this script.

Each round runs the same phases:
  warm       every hit.txt name once at 5,000 q/s (fills the caches)
  hit_max    hit.txt for 20 s, as fast as the server answers (cache-hit capacity)
  steady_2k  hit.txt for 30 s at a fixed 2,000 q/s (latency at a busy-household rate)
  miss_max   miss-<round>.txt for 15 s, as fast as possible (every query new: rules,
             upstream, learning, cache insert)
  idle       15 s without queries
For each phase it records dnsperf's summary and latency histogram, and on the SUT the
process CPU time (user+system, all threads), peak RSS during the phase (VmHWM after
resetting it) and RSS at the end.

--perf r1-hit_max,r3-miss_max,... also runs `perf record -g` (frame-pointer call graphs, event
--perf-event, default cpu-clock)
on the SUT process for the first --perf-seconds of those phases and keeps the data in
<--perf-dir>/<label>-<stamp>-<phase>.data on the SUT host. Profiled phases are marked
"profiled" in the report; their numbers include the profiler's small overhead.
"""

import argparse
import datetime
import json
import pathlib
import re
import shlex
import subprocess
import sys
import time

PHASES = [
    ("warm", "hit.txt", "-n 1 -Q 5000 -c 16"),
    ("hit_max", "hit.txt", "-l 20 -c 64 -q 2000"),
    ("steady_2k", "hit.txt", "-l 30 -Q 2000 -c 16"),
    ("miss_max", "miss-{round}.txt", "-l 15 -c 64 -q 2000"),
]
IDLE_SECONDS = 15


def ssh(host: str, command: str, timeout: int = 600) -> str:
    result = subprocess.run(["ssh", host, command], capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0:
        raise RuntimeError(f"ssh {host} failed ({result.returncode}): {result.stderr.strip()[:500]}")
    return result.stdout


def proc_sample(host: str, process: str, reset_peak: bool) -> dict:
    reset = 'echo 5 > /proc/$pid/clear_refs;' if reset_peak else ""
    out = ssh(host, (
        f"pid=$(pgrep -xo {shlex.quote(process)}) || exit 3; {reset}"
        "echo PID $pid; echo TCK $(getconf CLK_TCK); echo STAT $(cat /proc/$pid/stat);"
        "grep -E '^(VmRSS|VmHWM|Threads):' /proc/$pid/status"
    ))
    sample = {"time": time.time()}
    for line in out.splitlines():
        key, _, value = line.partition(" ")
        if key == "PID":
            sample["pid"] = int(value)
        elif key == "TCK":
            sample["tck"] = int(value)
        elif key == "STAT":
            # Fields after the ")" of comm: state is field 3; utime/stime are fields 14/15.
            fields = value.rsplit(")", 1)[1].split()
            sample["cpu_ticks"] = int(fields[11]) + int(fields[12])
        elif line.startswith(("VmRSS", "VmHWM", "Threads")):
            name, number = line.split(":", 1)
            sample[name] = int(number.split()[0])
    return sample


def parse_dnsperf(text: str) -> dict:
    def number(pattern: str):
        match = re.search(pattern, text)
        return float(match.group(1)) if match else None

    result = {
        "sent": number(r"Queries sent:\s+(\d+)"),
        "completed": number(r"Queries completed:\s+(\d+)"),
        "lost": number(r"Queries lost:\s+(\d+)"),
        "qps": number(r"Queries per second:\s+([\d.]+)"),
        "avg_latency_ms": None,
        "rcodes": None,
    }
    avg = number(r"Average Latency \(s\):\s+([\d.]+)")
    if avg is not None:
        result["avg_latency_ms"] = avg * 1000
    rcodes = re.search(r"Response codes:\s+(.*)", text)
    if rcodes:
        result["rcodes"] = rcodes.group(1).strip()
    buckets = [
        (float(high), int(count))
        for high, count in re.findall(r"^\s+[\d.]+ - ([\d.]+):\s+(\d+)\s*$", text, re.MULTILINE)
    ]
    total = sum(count for _, count in buckets)
    if total:
        for name, quantile in (("p50_ms", 0.50), ("p99_ms", 0.99), ("p999_ms", 0.999)):
            target, seen = quantile * total, 0
            for high, count in buckets:
                seen += count
                if seen >= target:
                    result[name] = high * 1000
                    break
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--load-host", required=True)
    parser.add_argument("--sut-host", required=True)
    parser.add_argument("--server", required=True, help="ip:port of the server under test, as seen from the load host")
    parser.add_argument("--process", required=True, help="exact process name on the SUT host (pgrep -x)")
    parser.add_argument("--label", required=True)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--load-cpus", default="0,1", help="taskset CPU list for dnsperf on the load host")
    parser.add_argument("--threads", type=int, default=2, help="dnsperf -T")
    parser.add_argument("--perf", default="", help="comma list of r<N>-<phase> to profile with perf record")
    parser.add_argument("--perf-seconds", type=int, default=10)
    parser.add_argument("--perf-freq", type=int, default=499)
    parser.add_argument("--perf-event", default="cpu-clock", help="cpu-clock works in VMs without a virtual PMU")
    parser.add_argument("--perf-dir", default="/root/bench/perf", help="output directory on the SUT host")
    args = parser.parse_args()
    profiled = {item.strip() for item in args.perf.split(",") if item.strip()}

    ip, _, port = args.server.rpartition(":")
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    out = args.out / f"{args.label}-{stamp}"
    out.mkdir(parents=True)

    report = {"label": args.label, "started": stamp, "rounds": []}
    initial = proc_sample(args.sut_host, args.process, reset_peak=False)
    report["initial"] = initial
    print(f"[{args.label}] pid {initial['pid']}, initial RSS {initial['VmRSS'] / 1024:.1f} MiB, threads {initial['Threads']}")

    for round_no in range(1, args.rounds + 1):
        phases = {}
        for name, corpus, options in PHASES:
            corpus = corpus.format(round=round_no)
            command = (
                f"taskset -c {args.load_cpus} ~/bench/dnsperf.sh -s {ip} -p {port} -d ~/bench/corpus/{corpus} "
                f"{options} -T {args.threads} -t 2 -O latency-histogram -O suppress=timeouts 2>&1"
            )
            before = proc_sample(args.sut_host, args.process, reset_peak=True)
            profiler = None
            key = f"r{round_no}-{name}"
            if key in profiled:
                data = f"{args.perf_dir}/{args.label}-{stamp}-{key}.data"
                profiler = subprocess.Popen(["ssh", args.sut_host, (
                    f"mkdir -p {shlex.quote(args.perf_dir)} && perf record -q -e {shlex.quote(args.perf_event)} -F {args.perf_freq} -g "
                    f"-o {shlex.quote(data)} -p {before['pid']} -- sleep {args.perf_seconds}"
                )], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
            text = ssh(args.load_host, command)
            if profiler:
                _, perf_err = profiler.communicate(timeout=args.perf_seconds + 60)
                if profiler.returncode != 0:
                    print(f"[{args.label}] perf record for {key} failed: {perf_err.strip()[:300]}", flush=True)
            after = proc_sample(args.sut_host, args.process, reset_peak=False)
            (out / f"r{round_no}-{name}.txt").write_text(text, encoding="utf-8")
            if after["pid"] != before["pid"]:
                raise SystemExit(f"{args.process} restarted during {name}")
            result = parse_dnsperf(text)
            cpu_seconds = (after["cpu_ticks"] - before["cpu_ticks"]) / after["tck"]
            wall = after["time"] - before["time"]
            result.update({
                "cpu_seconds": cpu_seconds,
                "cpu_cores_used": cpu_seconds / wall if wall else None,
                "cpu_us_per_query": cpu_seconds * 1e6 / result["completed"] if result["completed"] else None,
                "peak_rss_mib": after["VmHWM"] / 1024,
                "end_rss_mib": after["VmRSS"] / 1024,
                "threads": after["Threads"],
                "profiled": profiler is not None,
            })
            phases[name] = result
            print(
                f"[{args.label}] r{round_no} {name:9} qps {result['qps'] or 0:>9.0f}  lost {int(result['lost'] or 0):>6}"
                f"  p50 {result.get('p50_ms', 0):6.2f}  p99 {result.get('p99_ms', 0):6.2f} ms"
                f"  cpu {result['cpu_us_per_query'] or 0:6.1f} us/q  peak {result['peak_rss_mib']:6.1f} MiB",
                flush=True,
            )
        time.sleep(IDLE_SECONDS)
        idle = proc_sample(args.sut_host, args.process, reset_peak=False)
        phases["idle"] = {"end_rss_mib": idle["VmRSS"] / 1024}
        print(f"[{args.label}] r{round_no} idle      RSS {idle['VmRSS'] / 1024:.1f} MiB", flush=True)
        report["rounds"].append(phases)

    (out / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"[{args.label}] results in {out}")


if __name__ == "__main__":
    sys.exit(main())
