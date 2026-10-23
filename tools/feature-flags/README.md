# feature-flags differential (M2-1)

VM Linux root, Python 3 + PyYAML, iproute2, util-linux and OpenSSL. Builds and deploys nothing. Compares **BEFORE** (the schema-2
program, `v0.2.2`, with the old `config_rust`: `switch1`–`switch17`, values in `rule/switchN.txt`) with **AFTER** (the new program
with the schema-3 `config_rust`: `feature_flag`s) for many flag combinations. It reuses `tools/route-learner` (rig, replay agent,
row comparison) and `tools/e2e-1` (`prepare.py`, `mock.py`, `dnswire.py`).

```sh
python3 tools/feature-flags/run.py --inputs /private/inputs --work /private/run \
  --before /private/rtdns-before --after /private/rtdns-after --migrate /private/mosdns-settings-migrate --names 500
```

`inputs/` as for `tools/route-learner` (`config_all`, `config_rust` = OLD tree, `config_rust_new`, `snapshot/…`, outside the repository).
Both sides start from the same learned state (the snapshot's lists converted once into `gen/route_learner.json`; both programs use
`route_learner`). Every switch file of the snapshot is replaced by the combination's values, so no production value reaches a run.

| Combination | BEFORE switch files (others at the package defaults) | AFTER |
|---|---|---|
| `G0-defaults` | package defaults | switch files, migrated at start |
| `G1-common` | 3=B, 2=B, 12=A, 16=A (safe mode, listed clients direct, DDNS foreign; from the PRD) | `flags` in `settings.json` |
| `S<n>-<flag>` | switch n flipped (3, 17, 9, 8, 16, 1, 5, 7, 6, 4, 13) | alternately migrated / direct |
| `C1`/`C2`/`C3` | (2,12) = (A,B) / (B,A) / (A,A) | direct / migrated / migrated |
| `N1`/`N2` (negative controls) | defaults | `core_mode=safe` / `client_routing=listed_direct` deliberately: must show differing rows, else the comparison is blind |
| `H-hot_common` | defaults, then `POST /plugins/switchN/post` after pass 1 | defaults, then `POST /api/v1/flags/{key}` after pass 1 |

In `H` both sides then make the same extra settings apply (proxy override) before pass 2: a flag change is a whole-generation apply while the old switch POST was not, and in both programs a name learned during a generation carries its learned label in `domain_set` only from the next generation on (seen equally on both sides in every other combination). Without it, `H` differs in those labels only.

Each combination: pass 1 and pass 2 on port 53 (caches cold, then warm) and one pass on the re-query listener (3066, `sequence_requery`).
The corpus is e2e-1's (production ranking names × A/AAAA, controlled probes for block/white/grey/real/ddns/cnfilter/rewrite/adguard,
two client sources `127.0.0.1` and `127.0.0.2` with `127.0.0.2` in `client_ip.txt`, every listener port) plus SOA/PTR/HTTPS questions for
20 names and every probe. Compared per row, strictly: `rcode`, answers (no TTL), `domain_set`, `effective_tag`, `final_sequence`,
`final_upstream_group`, `matched_rule_source` (as a set); the learned sets after the run; AFTER's written `flags` must equal the expected
differences (an independent copy of the migration table, `flags.py`). Any difference, DNS error, missing audit row, a host that exits
or a leftover process/namespace fails. `summary.json` holds counts and corpus indexes only; raw rows stay in the work directory.

Tests: `python3 -m unittest discover -s tools/feature-flags`.
