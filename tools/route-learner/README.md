# route-learner differential (M2-2)

VM Linux root, Python 3 + PyYAML, iproute2, util-linux and OpenSSL. This tool builds and deploys nothing. It compares **BEFORE** (the previous `rtdns` + the old `config_rust`, learning through
`domain_output`) with **AFTER** (the new `rtdns` + the new `config_rust`, learning through `route_learner`) on the same corpus, mocks and learned state, and reports where DNS answers, audit routing
fields or learned sets differ. It reuses `tools/e2e-1` (`prepare.py`, `mock.py`, the audit correlation of `replay.py`, `dnswire.py`).

```sh
python3 tools/route-learner/run.py --inputs /private/inputs --work /private/run \
  --before /private/rtdns-before --after /private/rtdns-after \
  --migrate /private/mosdns-settings-migrate --names 1500 --groups ABDEF
```

`inputs/` holds `config_all` (only for e2e-1's preparation), `config_rust` (the OLD tree), `config_rust_new` (the NEW tree) and `snapshot/{rule,gen,srs,adguard,webinfo}` (production data;
read-only, never in the repository). `--work` must not exist. Every host runs with its mocks in its own loopback-only namespace; the original system `mosdns` service is not touched and its identity is checked
before and after (`cleanup.json`).

| Group | Start state | Compared |
|---|---|---|
| A cold start | no learned names on either side | pass 1 (learning while it runs) and pass 2, every row; learned sets after each pass |
| B learned state | BEFORE: the snapshot's `gen` lists + the controlled probe names; AFTER: the same names converted by `gen_to_store.py` (test-only; the product imports nothing) | initial sets, passes 1 and 2, sets after each pass |
| C restart (inside A) | A, then SIGTERM and start without cache dumps | the restart pass, and that both sides came back with exactly the sets they stopped with |
| D expiry | AFTER with every `ttl` 60 s; BEFORE cold | AFTER's learning pass against BEFORE's; then wait, flush caches, and AFTER's pass against BEFORE's cold pass (an expired state is a never-learned state); AFTER's sets empty |
| E SERVFAIL | cold; the `domestic` mock group answers SERVFAIL | sets: only no-V4/no-V6 names that BEFORE learned from a SERVFAIL answer may be missing from AFTER; rows: a differing row is *explained* only when its name is such a name |
| G requery flows | cold; the `domestic` mock group answers empty NOERROR (then `domestic`, `foreign`, `foreignecs`); the whole corpus is sent to the re-query listener (`sequence_requery`) | rows and learned sets of the re-query passes and of the normal pass that follows, strictly: this is where remembered no-V4, special names and the re-query remembered-proxy site learn |
| F hot applies | AFTER cold, then one pass with 6 settings applies running during it | no DNS errors, no failed apply, nothing learned is lost, RSS before/after |

Compared fields (every row, by pass and corpus index): `rcode`, answer records (TTL ignored), `domain_set`, `effective_tag`, `final_sequence`, `final_upstream_group`; the learned set of every category after each pass;
and `matched_rule_source` as a set. The learning pass of A, B, D and E is the only place `matched_rule_source` is relaxed: BEFORE shows a learned rule's name only for names whose rule file is already published into its provider,
and a name BEFORE learns during the run lives in an in-memory overlay with no source (the audit then shows the sequence rule's `inline:` identity) until the next publication hours later, while the learner always names its category.
There the five learned-rule names (`my_realiprule`, …) and `inline:` are dropped from both sides and a row that differs only by them is counted as `learner_source_only`. Every pass after the names were published
(the plan calls each category's `save` and then posts the written rule files into the `my_*rule` providers, which is what `domain_output`'s periodic write does a few hours into a normal run) is compared strictly:
the same set is required, the same parts in another order are counted as `order_only`. Any other difference, a DNS error, a missing audit row, a different learned set, a host that exits, or a leftover process/namespace fails the group.
`summary.json` holds counts and corpus indexes only; raw rows contain private names and stay in the work directory. For group B the preparation first makes the former lists consistent (every name of a category's statistics file
is in its rule file and the other way round, as `domain_output` writes them) so both sides start from the same names.

Tests: `python3 -m unittest discover -s tools/route-learner`.
