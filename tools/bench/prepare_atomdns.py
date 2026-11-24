#!/usr/bin/env python3
"""Copies an AtomDNS config root for the fixed benchmark and points it at the fake upstream.

  prepare_atomdns.py <source config root> <output dir> --upstream <host:port> --dns-port <port>

Changes, and nothing else:
  * every `addr:` of an upstream in sub_config/forward_*.yaml becomes udp://<upstream>
    (domestic, foreign, foreignecs, sing-box/mihomo FakeIP groups alike), `dial_addr:` and
    `use_proxy:` lines are dropped;
  * the DNS listeners `listen: ":53"` in config_custom.yaml move to `:<dns-port>`.
Rules, caches, audit capacity, learner and switches stay at the package defaults.
"""

import argparse
import pathlib
import re
import shutil

ADDR = re.compile(r'^(\s*)addr:\s*"[^"]*"(.*)$')
DROP = re.compile(r"^\s*(dial_addr|use_proxy):")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--upstream", required=True, help="fake upstream host:port")
    parser.add_argument("--dns-port", required=True, type=int)
    args = parser.parse_args()

    if args.output.exists():
        raise SystemExit(f"{args.output} exists; choose a new directory")
    shutil.copytree(args.source, args.output, ignore=shutil.ignore_patterns(".DS_Store", "ui"))

    rewritten = 0
    for path in sorted((args.output / "sub_config").glob("forward_*.yaml")):
        lines = []
        for line in path.read_text(encoding="utf-8").splitlines():
            if DROP.match(line):
                continue
            match = ADDR.match(line)
            if match:
                line = f'{match.group(1)}addr: "udp://{args.upstream}"'
                rewritten += 1
            lines.append(line)
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")

    main_config = args.output / "config_custom.yaml"
    text = main_config.read_text(encoding="utf-8")
    text, listeners = re.subn(r'listen: ":53"', f'listen: ":{args.dns_port}"', text)
    main_config.write_text(text, encoding="utf-8")
    print(f"{args.output}: {rewritten} upstream addresses -> udp://{args.upstream}, {listeners} listeners -> :{args.dns_port}")


if __name__ == "__main__":
    main()
