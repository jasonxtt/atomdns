# config-diff: routing differential between two default-config trees

Answers one question: *does this edit of the default configuration (`config_rust`) change what a
client sees, which upstreams are asked, or what the audit record says?* It is the safety net for
restructuring `sub_config/process_*.yaml` and `forward_1.yaml`.

```sh
# on the Linux test host (it starts the native binary on loopback; nothing is built or sent out)
python3 tools/config-diff/run.py --old <config_rust tree> --new <config_rust tree> \
    --binary <path to atomdns> --work <fresh dir> [--combos compat,safe+cnfake] [--only unl,wl] [--json out.json]
```

How it works

- Each tree is copied per flag combination (`core_mode` x `cn_fakeip` x `dns_mode` x `ddns_foreign` x
  caches on/off, see `FLAGS`) and started with `webinfo/settings.json` carrying those flags.
- Every upstream group is rewritten to five loopback mocks (domestic, foreign, foreign+ECS, FakeIP,
  domestic FakeIP). The `.srs` rule sets are replaced by small synthetic lists, so any mark
  (blocklist, ddns, grey/white, realip, cnfakeipfilter, cuscn, cusnocn, geosite_cn/no_cn and
  combinations of them) can be reached by choosing the domain.
- A name encodes the answers: `<domestic>.<foreign>.<class>-<n>.<class>.test`, e.g.
  `cn.none.unl-m1-5.unl.test` = unlisted name, domestic answers a CN address, foreign answers
  nothing. Each name is asked twice (second time it meets caches and the learned routes), as A and
  AAAA, through the main entry and through `sequence_requery` (a temporary extra listener).
- Per query it records the client response (TTL ignored), the mocks asked in order of first contact
  and `final_sequence / final_upstream / domain_set / effective_tag` from the audit log, and prints
  every case that differs between the trees.

Reading the result

- Compare against the *previous* tree to prove a refactor changes nothing. Renamed sequences show up
  as `final_sequence` differences only; filter them (`analyze` snippet in the commit message of the
  restructuring) before concluding.
- For a policy change (safe mode) look at `asked`: the invariant checked in 2026-11 was "in safe mode
  no unlisted name reaches the domestic mock; only explicit lists (whitelist, cuscn, geosite_cn,
  cnfakeipfilter with cn_fakeip, DDNS without `ddns_foreign`) do".
- The default 10 flag combinations x ~16 list classes take about five minutes.
