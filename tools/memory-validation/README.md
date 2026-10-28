# Isolated native memory validation

These tools implement the approved lossless-index/audit research plan. They
never install a service, modify production, merge, or certify the whole project.

## Hot-apply and audit operation peaks (2026-10-26)

`operation_peaks.py` measures M0/M1/M3/MF (500 qps, 150 s, snapshots/reset at
35 s and 145 s, then 300 s idle) and full-100k audit read/clear/resize operations.
Three pairs alternate B/C,C/B,B/C. Use root on the isolated Linux test host;
CoreDNS, both ordinary binaries, and frozen configuration must already exist.
No download or service reset occurs. Output root must be new and private.

```sh
python3 tools/memory-validation/operation_peaks.py init --root NEW_PRIVATE_ROOT --source PRIVATE_FROZEN_CONFIG --base BASE_BINARY --candidate CANDIDATE_BINARY --core PINNED_COREDNS
taskset -c 3 python3 tools/memory-validation/operation_peaks.py run --root NEW_PRIVATE_ROOT
```

The loopback-only namespace has no default route. Validate every selected
supplier using its actual UDP/DoH protocol before starting the SUT. SUT CPU0,
sender CPU1, CoreDNS/proxy CPU2, independent sampler CPU3. Settings saves preserve
other overrides; six flag changes restore the original bool value without requery.
Actual SUT port-40053 drops and namespace UDP error counters are read separately.
Operation latency windows retain the earlier 15 s definition.

The user explicitly approved separating the known underscore-CNAME SERVFAIL
queries (frozen corpus indices 3972,3973,5187); their names are verified by hashes
before using those exceptions. Every query stays in the workload and raw output;
known failures never count as correct. Any other protocol error, loss, failed
operation, missing snapshot or failure to restore the setting stops the matrix.
No complete DNS semantic oracle or whole-project correctness PASS is implied.
The ordinary sampler's legacy Drops53 field is not used for this fixture.

`operation_summary.py` gates complete pairs by role medians, all-run HWM cap
(hot applies only) and same-pair/role M0 idle RSS. An incomplete or failed attempt
is retained and never replaced. Raw replies, private runtime copies and manifests
must remain outside Git. Archive them privately before removing the owned
processes/namespaces and newly created remote task directories.

For the explicitly approved MF/R3 continuation, restore the exact prepared
`work/`, `upstream/` and binaries into a new private root; keep the original
report as `prior-report.json`. Its M0 controls must match all binary, input,
corpus and CoreDNS hashes and contain three successful pairs. This reuses the
earlier matching elapsed-time snapshots; it does not rerun M0 at the new wall
time. Natural learner expiry remains visible. The old batch is never overwritten.

```sh
taskset -c 3 python3 tools/memory-validation/operation_peaks.py run --root NEW_PRIVATE_ROOT --scenarios MF R3 --prior-report NEW_PRIVATE_ROOT/prior-report.json --ignore-unsent
```

`--ignore-unsent` is opt-in and requires explicit user approval. It preserves
raw counts and records how many unsent slots were ignored, while timeouts,
send errors, unexpected DNS responses, socket drops, operation failures and
memory limits still stop the run. The sender and its 20ms skip policy are
unchanged. Default runs retain the original strict coverage gate.

Run on a Linux VM with Python/PyYAML, Rust, Node, OpenSSL, iproute2, taskset and
socat (browser proof only). Reuse `tools/5d-1`, `5d-2` and `e2e-1` helpers.
Source configs, raw DNS/HTTP results, corpus, certificates and per-file manifests
must remain outside Git. Inspect input mapping before using a new config.

1. Build BASE and candidates with `scripts/build-rust-native.sh`, using separate
   output files. Keep all dependency versions identical.
   Copy-based builds must use source-keyed targets. A reused artifact caused an
   initial diagnostic failure in this study: never accept a suspiciously instant
   build or identical BASE/candidate byte hash without checking provenance.
2. `prepare.py --source PRIVATE_CONFIG --output NEW_PRIVATE_PREPARED` copies the
   current native config, maps known upstreams to deterministic E2E peers, enables
   audit capacity100000 and generates local TLS credentials. An empty package's
   upstream lists get explicit fixtures; production inputs remain untouched.
   `--audit-capacity 0|100000|400000` makes a separate fingerprinted test fixture.
   Unknown upstream groups reject preparation rather than contacting them.
3. `run.py --work PREPARED --output NEW_ORACLE --binary base=BASE --oracle-only`
   characterizes the fixed weighted `gen/top_domains.txt` corpus. An explicit
   `--top PRIVATE_TOP_FILE` is available. Identity records the raw source SHA-256
   and `top_source`; every explicit source is automatically marked supplementary
   by the evaluator. Missing provenance makes old runs incomplete.
4. Verify the runner first with `--smoke`. Formal timing is currently blocked
   whenever `route_learner` is configured, even with an empty saved snapshot:
   expiry and in-run learning invalidate a static oracle. Preparation records
   this capability; legacy manifests with unknown capability must be re-prepared. The final BASE/combined smoke exposed this
   exact issue. A state-aware oracle/fixture plan must be reviewed before these
   production-snapshot matrices run; do not disable the guard or waive cells.
   Intended formal usage after that repair:

   ```sh
   python3 tools/memory-validation/run.py --work PRIVATE_PREPARED \
     --output NEW_PRIVATE_RUN --binary base=BASE --binary candidate=CANDIDATE \
     --oracle PRIVATE_ORACLE/oracle.json --scenario steady
   python3 tools/memory-validation/evaluate.py NEW_PRIVATE_RUN
   ```

   Formal runs require exactly two roles (BASE then candidate). The frozen
   schedule is B/C,C/B,B/C with seeds501–503; each process is pinned to CPU0,
   mock/sender to CPU1. Prefill matches the frozen audit capacity, followed by
   60s warmup +300s measured at500QPS. All planned prefill queries must be
   sent, healthy and correct before formal measurement starts; the per-run
   `prefill-summary.json` is independently checked again by the evaluator. Other scenario choices: settings, groups,
   reads, reload. Six management applies or a fixed diagnostic-read schedule
   run within the measured window. Original DNS response semantics are checked;
   only TTL is normalized, with the existing bounded-TTL correctness check.
   Every outcome and management failure remains evidence. CPU/RSS/FD/thread
   samples and original requests are retained (request JSONL is gzip-compressed
   losslessly after analysis). PSS is recorded at the outer memory points.
   Measured HWM resets at the end of warmup; initial HWM is recorded separately.
   Keep builds, other test jobs and browser probes out of formal timing.
5. `diagnostic_candidate.py --source SOURCE --output NEW_DIAGNOSTIC_SOURCE`
   injects the established original-capacity audit accounting and shutdown
   launcher. Build `mosdns-native-host --example audit_memory --release --locked`
   with a fresh target (or explicitly clean/rebuild the native-host package
   between source copies). `run.py --diagnostic` fills the audit ring and exits
   after a short observation; its CPU/latency/RSS are not acceptance evidence.
   For a causal retained-heap comparison, add `--synthetic`. This injects the
   same in-process sequence into a private unit-test copy and fixes the sharing
   hint hasher to DefaultHasher **only in both diagnostic copies**. RandomState
   collision mappings otherwise vary retained sharing between identical runs.
   `memory-diagnostic.json` records the transformation and fixture hash. Run:

   ```sh
   RTDNS_MEMORY_AUDIT_CAPACITY=100000 RTDNS_MEMORY_AUDIT_PROFILE=ttl \
     cargo test --locked --manifest-path NEW_DIAGNOSTIC_SOURCE/rust/Cargo.toml \
       -p mosdns-native-host --lib memory_validation_synthetic_retention -- --nocapture
   ```

   Profiles are `shared`, `ttl` and `unique`; capacities are0/100000/400000.
   Compare identical input checksums and all record/template/response populations,
   then requested heap. These fixtures do not establish production workload
   representativeness or CPU/latency/RSS gates. Keep separate executable copies,
   force/review native-host recompilation between source trees, and record hashes.
   `index-probe/main.rs` is a separate counting-allocator executable taking rule
   files/SRS paths; its constructed provider metadata exercises complete valued
   results but does not represent a whole production mapper allocation census.
   Set diagnostic-only `RTDNS_MEMORY_PROBE_OUTPUT` to a PRIVATE path for full
   query/result byte comparison (emitted after measurement, never published).
6. `browser.py` serves a copied runtime through a loopback proxy for up to600s,
   or until its output `stop` file exists. Connect an owned SSH forward and test
   the actual UI controls. Every owned process/namespace has a cleanup receipt.

`evaluate.py` refuses incomplete pairs, bad/skipped DNS, non-full rings, failed
management/cleanup, diagnostic timing and missing data. A scenario PASS says
nothing about unrun scenarios, heap goals, independent review or the two-hour
soak. The current runner does not implement that soak or cache-capacity stress.
The production snapshot has seven20-million-entry cache limits: no maximum-fill
claim is made. Any scaled cache stress requires an explicitly separate fixture
and an agreed scope; never silently shrink the production comparison's limits.

Tool checks: `python3 -m unittest discover -s tools/memory-validation -p 'test_*.py'`.

The newer `ValidationEnv` wrapper also fingerprints all existing product processes
by PID/start/image identity, including the test host's `mosdns-rust`; the legacy
harness's `mosdns`-unit receipt is insufficient by itself. The evaluator requires
both cleanup and this existing-process guard. The current production snapshot's
static-oracle aging failure is retained as a blocker; no formal run is launched
with that unsupported model.

## Experimental K1 closed-loop pilot (not qualified)

`state_replay.py` is a separate research pilot, not a replacement for run.py.
It runs BASE/control(BASE)/candidate sequentially in new ValidationEnv copies,
uses two identically ordered corpus passes, and captures per-query learner
before/after state, DNS and complete terminal audit. Only the documented routing
projection and logical learner fields are compared; timestamps and raw output
remain private. Missing/ambiguous evidence cannot qualify a trace. No timing
metrics or formal-run authorization is produced. Use new absolute private paths:

```sh
python3 tools/memory-validation/state_replay.py --work PRIVATE_PREPARED \
  --output NEW_PRIVATE_PILOT --base BASE --candidate COMBINED \
  --limit 128 --include-index 1267
```

Omitting --limit selects the full current corpus; every limited run is
supplementary. The recorded pilot used137 questions twice per role and exposed
BASE/control final_upstream instability. It is not accepted as a state-aware
formal oracle; see the [consolidated qualification limits](../../docs/rust/validation-records/10-25-memory-optimization/README.md#correctness-and-qualification-limits). Never ignore the differing
supplier field or bypass the existing configured-learner guard to obtain PASS.

## Same-config real-upstream observation

At the user's direction, `real_config_probe.py` preserves production aliapi/proxy
settings rather than remapping peers to mock. It copies a fresh source, checks
complete effective settings equality, remaps only its own listener/API/self-call
ports, and runs separate cold-cache processes with private/tmp. Requires root,
socat and free reserved loopback ports; non-empty dedicated groups reject until
an explicit port mapping is added. `--production-host` supplies the LAN address
of the source localdns:5533; test-owned UDP/TCP relays provide that production
service at test loopback5533. Both roles use that relay, whose remote client
identity differs from the production loopback client. Existing1053/5533 or owned
port occupants reject; only closed TCP TIME_WAIT is reusable.

```sh
python3 tools/memory-validation/real_config_probe.py --source PRIVATE_PRODUCTION_COPY \
  --output NEW_PRIVATE_OBSERVATION --base BASE --candidate COMBINED \
  --production-host PRODUCTION_LAN_IP --fill-count 100000 --settle 10
```

The first128 production questions are used twice for captured short traces, then
cycled for optional100000-query fill. Full raw DNS fill records are gzip-preserved.
Real observation accepts the DNS wire TTL range; state_replay's mock default
stays300 seconds. No static oracle or formal-run guard is bypassed. NOERROR is
not asserted to mean correct. Output is deliberately not acceptance eligible:
no60/300s performance window, three pairs, production workload census or soak.
Use identical BASE as both artifacts for an external-variation control. Shutdown
checks original service identities and remaining owned process-group members.

The same-config observer also accepts `--timed-spec PRIVATE_JSON` and `--reverse`.
A timed spec explicitly fixes warmup/duration/rate/seed/idle; the current long
observation schedule is60s/300s/500QPS/seed501..503/30s. The initial128-question
prefill remains, but timed load uses the complete production corpus by weight.
`fload.py`'s opt-in `observation_only` mode preserves every decoded reply in gzip,
labels protocol statuses as observed_* (never correct), and retains skipped,
malformed/question-mismatched, SERVFAIL and timeout outcomes. Existing oracle
classification/default300 TTL and the formal runner/evaluator are unchanged.
CPU is per protocol reply, including SERVFAIL; it is not per correct query.
Warmup HWM reset is an event with a retained before-reset memory snapshot.
The legacy connection/kernel UDP samples describe the host network namespace;
Drops53 is not the new40053 listener and is not used as its drop counter.
A paired observation still cannot certify correctness or whole-project acceptance.

Recompute the frozen three-pair aggregate with `real_pair_summary.py r1/report.json
r2/report.json r3/report.json`. It checks schedule/order/identity/full-ring/cleanup,
keeps per-run outcomes and medians, and reports all-run maximum HWM. Its
observations_complete status does not establish correctness or project acceptance.
