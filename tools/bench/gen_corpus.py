#!/usr/bin/env python3
"""Deterministic dnsperf query files for the fixed AtomDNS benchmark.

Three kinds of names, so every routing branch is exercised:
  cn        names from a China domain list (one domain per line, `full:`/`regexp:` lines skipped)
  foreign   subdomains of well-known non-China domains (all are in geolocation-!cn)
  unlisted  unique second-level .com names that no shipped list contains

Outputs (in --out):
  hit.txt       10,000 fixed names; after one warm-up pass every query is a cache hit
  miss-1.txt .. miss-N.txt   unique names per round, never repeated across files

85% of queries are A and 15% AAAA. The same seed always yields the same files.
"""

import argparse
import pathlib
import random

FOREIGN_BASES = [
    "google.com", "youtube.com", "googlevideo.com", "gstatic.com", "googleapis.com",
    "facebook.com", "instagram.com", "whatsapp.net", "twitter.com", "x.com",
    "github.com", "githubusercontent.com", "wikipedia.org", "reddit.com", "netflix.com",
    "nflxvideo.net", "telegram.org", "discord.com", "twitch.tv", "amazonaws.com",
    "cloudfront.net", "openai.com", "chatgpt.com", "anthropic.com", "medium.com",
    "stackoverflow.com", "dropbox.com", "spotify.com", "pinterest.com", "tumblr.com",
    "vimeo.com", "quora.com", "yahoo.com", "bbc.co.uk", "nytimes.com",
    "duckduckgo.com", "protonmail.com", "imgur.com", "steamcommunity.com", "docker.io",
]


def cn_domains(path: pathlib.Path) -> list[str]:
    names = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or ":" in line:
            continue
        names.append(line)
    return names


def qtype(rng: random.Random) -> str:
    return "AAAA" if rng.random() < 0.15 else "A"


def pick(rng: random.Random, kind_weights) -> str:
    roll = rng.random()
    for kind, upper in kind_weights:
        if roll < upper:
            return kind
    return kind_weights[-1][0]


# 40% China-listed, 30% foreign-listed, 30% unlisted.
WEIGHTS = [("cn", 0.4), ("foreign", 0.7), ("unlisted", 1.0)]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--cn-list", required=True, type=pathlib.Path, help="China domain list, e.g. rule/geosite_cn.txt")
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--hit-names", type=int, default=10_000)
    parser.add_argument("--miss-names", type=int, default=600_000, help="unique names per miss file")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--seed", type=int, default=20261124)
    args = parser.parse_args()

    rng = random.Random(args.seed)
    cn = cn_domains(args.cn_list)
    args.out.mkdir(parents=True, exist_ok=True)

    hit = []
    cn_sample = rng.sample(cn, k=min(len(cn), args.hit_names))
    for index in range(args.hit_names):
        kind = pick(rng, WEIGHTS)
        if kind == "cn":
            name = cn_sample[index % len(cn_sample)]
        elif kind == "foreign":
            name = f"w{index}.{rng.choice(FOREIGN_BASES)}"
        else:
            name = f"zqh{index}x.com"
        hit.append(f"{name} {qtype(rng)}")
    (args.out / "hit.txt").write_text("\n".join(hit) + "\n", encoding="ascii")

    for round_no in range(1, args.rounds + 1):
        lines = []
        for index in range(args.miss_names):
            kind = pick(rng, WEIGHTS)
            label = f"m{round_no}n{index}"
            if kind == "cn":
                name = f"{label}.{rng.choice(cn)}"
            elif kind == "foreign":
                name = f"{label}.{rng.choice(FOREIGN_BASES)}"
            else:
                name = f"zq{label}x.com"
            lines.append(f"{name} {qtype(rng)}")
        (args.out / f"miss-{round_no}.txt").write_text("\n".join(lines) + "\n", encoding="ascii")
    print(f"wrote {args.out}: hit.txt ({len(hit)}), {args.rounds} miss files ({args.miss_names} each)")


if __name__ == "__main__":
    main()
