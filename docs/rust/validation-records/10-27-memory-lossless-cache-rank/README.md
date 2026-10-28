# Lossless cache and rank memory optimization

Updated 2026-10-28. **The three implementations are withdrawn from the merge
candidate.** Rust sources and all247 program build inputs have been restored
exactly to the pre-increment `3687d269` baseline by withdrawal commit `0868fded`;
fresh Linux workspace1533/0/0,formatting and strict Clippy pass. Prior domain-index/audit work is
preserved. No main merge, push, release or production installation occurred.

[The attribution follow-up](diagnosis-2026-10-28.md) retains identical-BASE
controls: CPU+7.92%,p95+5.06%,p99+16.49% with unchanged source and seed. These
demonstrate measurement instability, not candidate safety. Management-read peak
gains do not establish lower retained/steady memory; whole-program safety remains
unproven. The owner required exclusion if this could not be resolved. Prior
candidate data and review prompts below are historical evidence, not an active
merge/review submission or a passing acceptance result.

## Scope and source identity

This is an incremental comparison against `3687d269b1699b7061a3b48c5e4a9f51745bc625`,
which already includes the earlier accepted domain-index/audit work and subsequent
managed-upstream and exact-domain-count changes. It is not a fresh comparison
against main. The earlier CPU exception does not waive this increment's 5% check.

Frozen product candidate: `fb61c2474e663e39c88e1aa55831eb8006731bfb`.
Method v1: `baab79fa17059f4db35aa98f83cc6944b64af639`; method v2:
`7337dc6309b451279e285ff129daffc142879c4e`, plus the fingerprinted,
mechanically derived two-CPU profile. Later evidence-only commits do not change
the tested Rust source.
The separate closed-loop read matrix is frozen at
`cbff46d20f64cb24b15e09174269ee30db741f75`.
The subsequent full observation collection is frozen at
`7b72bba7f330c60aca1005cc77be0c78981a78cd`.

| Commit | Behavior |
| --- | --- |
| `a9c4447a` | Validate and patch ID/RA in the request-private TTL-aged cache response; remove its second wire copy. |
| `93410152` | Select exact top-K before formatting labels; retain full-snapshot distinct-domain counting. |
| `fb61c247` | Capture immutable shared cache entries for show/dump/save; avoid copying all cached payloads. |

No dependency, YAML, API body, persistence format, retention limit, TTL policy or
default configuration change. Full audit answers, provenance, Trace IDs, client
addresses and the independent 300 slowest records remain. No tiered/lossy logging,
learner pagination, allocator replacement or unrelated CNAME repair is included.

## Correctness and lifecycle

- Cache headers: byte preservation outside ID/RA, validation before mutation,
  repeated request IDs, fresh/lazy TTLs and immutable cached originals.
- Rank selection: full-sort reference for all four grouped ranks; zero, small
  and extreme K; raw-domain versus grouping order; textual IPv4/IPv6 tie order;
  full distinct counts; cancellation and existing two-worker admission.
  Only three labels are formatted for K=3 among 10,000 groups. Borrowed input
  keys still require O(N) space; selected/output rows use O(K).
- Shared snapshots: payload addresses are shared; replacement and flush retain
  coherent historical views; the last view releases its references. Capture-time
  expiration, exact logical export budgets, owned API compatibility and v2 bytes
  are covered. Import still fully validates before publication. Show parses each
  visited row as before, searches complete answers, and renders only selected rows.
- Standard Linux workspace: **1541 passed, 0 failed, 0 ignored**. Formatting,
  strict workspace/all-target Clippy and both standard-script builds succeeded.
- macOS targeted checks succeeded except the pre-existing disconnect-saturation
  assertion in `v2_stats_windows_and_logs_use_retained_real_dns_records`: actual
  HTTP200 versus expected503. A single diagnostic repeat failed identically; an
  independently archived, freshly built unchanged `3687d269` also failed at the
  same assertion. No test or slot policy was weakened. Linux passed this test.
  This record does not claim a complete macOS PASS.

The user redirected testing from mos-test to **mosdns-rust** (two CPUs, about8GiB
RAM) after disk pressure. The disk-full and user-stopped attempts remain partial.
An old unrelated test target temporarily archived on mos-test was restored to its
original path after verifying all3374 file hashes. Existing services were protected.

## Requested-allocation diagnostic

The separate `lossless_probe.py` System-allocation binary is not the ordinary
timing executable. It checks 1000 fresh and lazy hits and 2000-entry small-A and
4145-byte-TXT snapshots/dumps. Allocation totals and live requested-byte deltas
exclude allocator rounding/metadata and residency; they are not RSS or retained
production-heap estimates. The full rows and identities will be published in
`evidence.json` after final collection.

Observed Linux results include exactly one response buffer fewer per hit:
53,000 fewer requested bytes across1000 small-A hits and4,145,000 fewer across1000
large-TXT hits. For2000 large-TXT entries, snapshot peak requested-byte delta
fell8,682,964 ->33,808 B; v2 dump fell9,130,356 ->458,516 B. Independent stores can
iterate in different orders; dump hashes are not an equality oracle between them.
The codec test compares owned/shared encodings of the same captured order.

## Whole-process observations

Both timing batches stopped on missing sends; there is no qualified CPU/latency
comparison. All planned roles use standard script release builds on the same
host, the frozen production configuration/rules and production-derived corpus snapshot, and one
fixed CoreDNS supplier per business group. The namespace has no default route.
Only endpoint/proxy/listener paths are remapped in copied inputs. All configured
fixture protocols are zone-checked before each SUT starts; this is40 protocol
combinations, not a new exhaustive91,872-question infrastructure qualification.

The schedule is B/C,C/B,B/C with seeds501..503,100000 retained audit records,
500QPS,60s warmup/300s measurement/30s idle. SUT usesCPU0; CoreDNS, proxy, sender
and the nice+10 sampler shareCPU1. V2 sets only the timed sender to nice-5;
the sampler explicitly keeps absolute nice+10 instead of inheriting a relative
offset. SUT, CoreDNS, proxy and post-read worker retain nice0. A separate live
200-question smoke returned200/200 and confirmed sender affinity1/nice-5 and
sampler affinity1/nice+10; five method tests passed locally and on Linux.
This profile cannot be numerically spliced
with the earlier four-CPU mos-test measurements. Role medians determine ratios;
all individual results and the largest observed peak must be retained.

The known underscore-owner/CNAME failure remains visible and was explicitly
deferred by the user. The denominator is **protocol replies**, including failures,
not verified-correct queries. Per-sequence semantic comparisons and natural
learner/TTL differences must remain in the report. Resource observations alone
do not close the original full correctness/state or production-heap gates.

After each DNS/idle window, four ranks and each of seven caches' show/page/dump
are read three times (75 reads per role). Each read resets kernel HWM immediately
before the operation and captures RSS/HWM after completion. HTTP statuses, rank
shape, show framing and gzip framing/CRC are checked; raw bodies stay private.
These are management read peaks, separate from DNS CPU/latency, startup and
hot-application peaks. They do not establish complete content equality across
independent wall-clock states. No two-hour soak or new hot-apply matrix is claimed.

### Retained failed first batch

`observations-v1` stopped after its first BASE role:150000 scheduled,
149996 sent/replied,149989 NOERROR,7 known SERVFAIL,4 unsent. Skipped sequences
170238..170241 follow a28.747ms actual-send gap at about340.476s of the360s
session. All observed namespace UDP errors/socket drops and orphan packets were
zero; all75 post-reads returned200 and validated. Cleanup and existing-service
identity guards passed. No candidate role ran, so this is not a valid pair.

Nearby sampled CPU usage was low; this does not establish the cause of the gap.
V2 prospectively changes sender priority to reduce competing scheduling and
keeps all coverage thresholds. It is a separate batch, not a repair/reclassification
of the failed result. No unsent waiver or product change is included.

V2 also stopped after its first BASE:149999 sent/replied,149992 NOERROR,
7 known SERVFAIL,1 unsent (sequence94639 at189.278s). The actual-send gap was
20.743ms. Priority did not resolve dispatch coverage; no candidate ran in either
batch. Independent replay of all179999 raw response lines found maximum gzip
write time about2.0ms at level9 and0.42ms at level1. Decoded hashes were identical;
this standalone replay did not reproduce the live stall and does not establish
its cause. No compression change was selected for acceptance.

A subsequent instrumented candidate diagnostic received all180000 replies.
During its timed window, GC pauses were about1ms or less; the largest recorded
decode was3.39ms. A select wait took18.079ms wall time with0.062ms thread CPU;
gzip write took16.731ms wall with5.725ms CPU. These observations do not establish
the cause of the earlier missed sends, and the diagnostic is not an acceptance
run. No GC, compression or product scheduling change was selected.

Method `7b72bba7 --collect-incomplete-sends` collected all three pairs.
It changes only collection after a completed role with unsent-only shortfall:
every sent query must have replied, raw rows must be complete and orphan/other
errors absent. All process/config/read guards remain. The default still stops;
the original summary still rejects any packet shortfall. If needed, a separately
named `unqualified_descriptive_metrics` reports numbers without changing that
verdict. Collecting later roles does not waive the coverage or5% checks, and does
not replace either stopped batch. Three performance-summary tests and five
lossless-method tests pass locally and on Linux.

### Completed timing collection: unqualified descriptive results

All900000 scheduled measurement slots remain visible:899929 sent/replied,
71 unsent (all candidate),24 known SERVFAIL included in replies, and zero orphan
packets. Every sent query received a well-formed question-matched reply;
450 post-load HTTP reads passed. All rings, protected inputs, cleanup and existing
service guards passed. All606 raw files referenced by role receipts were locally
rehash-verified; port40053 socket-drop snapshots are zero in all six roles.

| Pair/order | Role | Replies | Unsent | CPU us/protocol reply | RSS median KiB | p95 ms | p99 ms |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 B/C | BASE | 150000 | 0 | 160.533 | 64190 | 0.688761 | 2.766301 |
| 1 B/C | Candidate | 149942 | 58 | 179.069 | 63938 | 0.833254 | 3.706256 |
| 2 C/B | Candidate | 149992 | 8 | 219.678 | 64178 | 0.894239 | 3.231675 |
| 2 C/B | BASE | 150000 | 0 | 217.600 | 64048 | 0.943069 | 3.247418 |
| 3 B/C | BASE | 150000 | 0 | 197.800 | 65878 | 0.787503 | 2.704814 |
| 3 B/C | Candidate | 149995 | 5 | 193.340 | 65580 | 0.783950 | 2.649539 |

Compute each role's three-run median first, then the candidate/BASE ratio, exactly
as frozen. Do not switch to the median of paired percentages after seeing results.

| Descriptive metric | BASE median | Candidate median | Change | Qualification |
| --- | ---: | ---: | ---: | --- |
| RSS KiB | 64190 | 64178 | -0.02% | No repeatable steady RSS reduction established |
| CPU us/protocol reply | 197.800 | 193.340 | -2.25% | Unqualified; not a CPU benefit claim |
| p95 ms | 0.787503 | 0.833254 | +5.81% | Descriptive value exceeds5% |
| p99 ms | 2.766301 | 3.231675 | +16.82% | Descriptive value exceeds5% |

Original summary remains **incomplete: packet shortfall** in three candidate
roles. Descriptive latency also exceeds the selected5% screen. The first pair
worsened; pairs2/3 have smaller or negative latency changes. Absolute CPU varies
substantially across runs/seeds. These data do not establish a specific source
regression or its absence. Planned-to-receive latency includes sender dispatch
delay; it is not isolated server execution time. Two-CPU auxiliary co-location,
unverified host/frequency conditions and live state/time remain possible confounds,
not proven causes. Cache periodic-save interval is36000s, so no periodic export
falls inside these360s sessions; explicit diagnostic reads happen afterwards.

For449929 questions present in both roles, paired DNS projections have zero
differences.71 candidate questions have no comparison. Projection excludes TTL,
flags and RR ordering; raw wire retains them. This does not certify full routing/
learner state or turn the24 known failures into correct responses. The independent
read-content experiment compares exact stored wire and origin instead.

Full numerical values, all role receipts, hashes and failed batches are in
[sanitized machine-readable evidence](evidence.json). No gate was waived and no
Rust product change was made during measurement.

### Separate closed-loop read matrix

Method `cbff46d2` uses all11476 production questions in fixed order, repeating to
100000 closed-loop replies, followed by75 HTTP reads. There is no open-loop
timing sender or periodic resource sampler in this matrix. SUTCPU0 and auxiliary
workers/CoreDNS/proxyCPU1 remain; all have ordinary nice0. Three pairs use
B/C,C/B,B/C and identical frozen inputs/binaries. All600000 replies were retained:
599838 NOERROR and162 known SERVFAIL (27 per role). Every ring held100000 records;
all450 reads, protected-input checks and cleanup/service-identity guards passed.
This completes the read experiment, not the separate DNS timing gate.

Each cell below is the median of three per-run maxima across operations in that
category. Increment means HWM after one read minus RSS immediately before it;
it is a resident-memory observation, not retained heap or an allocator estimate.

| Read category | BASE HWM KiB | Candidate HWM KiB | BASE increment KiB | Candidate increment KiB |
| --- | ---: | ---: | ---: | ---: |
| Four ranks | 71864 | 72040 | 2728 | 2432 |
| Cache show | 75676 | 70180 | 6148 | 324 |
| Cache page | 75688 | 70180 | 3008 | 256 |
| Cache dump | 75100 | 70856 | 2368 | 896 |

Rank absolute HWM did not fall; its0.24% increase does not establish a meaningful
RSS regression or reduction. Show's observed incremental peak fell94.73%, and
dump's62.16%. These figures cannot be substituted for steady-state RSS savings.

| Pair | BASE all-read maximum KiB | Candidate all-read maximum KiB | BASE largest increment KiB | Candidate largest increment KiB |
| --- | ---: | ---: | ---: | ---: |
| 1 | 75584 | 72076 | 6148 | 2432 |
| 2 | 75688 | 72040 | 6148 | 2368 |
| 3 | 76084 | 71940 | 6148 | 2432 |

The largest HWM over every read/run was76084 ->72076 KiB (-5.27%). No startup,
hot-apply or steady-state RSS claim is inferred from this matrix.

All repetitions' raw identities were checked.36 rank comparisons and126
show/page comparisons matched; show comparison excludes its three absolute
timestamp lines.63 dump comparisons covered180711 entries (60237 for the first
repetition alone): key sets, exact DNS wire including ID/TTL, labels, origin
bytes and both expiration intervals matched. Only absolute stored/expiry times
and iteration order were normalized. See
[read-only comparison script](compare-read-outputs.py); raw bodies stay private.

## Live browser workflow

The unchanged current Vue bundle was built locally and copied into a separate
candidate-only preflight. That preflight alone binds0.0.0.0:49099 inside its
isolated namespace, exposed through a loopback SSH tunnel; measured runs retain
their loopback listener. All owned preflight processes, namespace and tunnel
were removed before the timed batch.

Real UDP and TCP supplied528 replies. The browser verified seven cache owners;
100 initial rows ->127 after loading the next page; answer search returning22
rows whose CNAME targets contain the query; complete DNS markers, CNAME, TTL and
metadata; query details including Trace ID; exact distinct count68 and a rank
detail count8. Audit capacity changed100000 ->1000 ->100000 and cleared existing
records;528 new DNS records were then added and cleared. Clearing all seven
caches showed zero entries for every owner. Preflight HTTP reads were75/75.
These workflow checks are not an oracle for every production DNS answer.

## Reproduction and evidence locations

Private host root: `/root/rtdns-lossless-20261027` on `ssh mosdns-rust`.
It contains exact Git archives, full frozen inputs, binary/build manifests,
Linux check logs, allocation probes, derived method receipt, raw DNS/resource
observations and post-read bodies. No credentials or raw query bodies belong in
the public repository. Sanitized identities and results are in `evidence.json`;
raw evidence remains available to the external reviewer. Local integrity checks
verified606 timing-role files against their receipts. Both source/build manifests
and all138 derived Python-file hashes match their frozen Git archives.

See [exact-host reproduction commands](REPRODUCE.md) and
[diagnostic method](../../../../tools/memory-validation/LOSSLESS-READS.md).

## Review, rollback and remaining gates

External review and user acceptance are pending. Each product slice can be
reverted independently in reverse order, followed by the affected checks and
the combined comparison. The old owned snapshot APIs and persistence format
remain available. Keep this branch separate from main. This task does not
authorize publication, service replacement, merge or production access.

The owned upstream is now inactive and startup-disabled; its installation and
all private source/binary/evidence directories remain for reproduction. All owned
case/upstream namespaces are absent. The pre-existing mosdns service retained its
original PID827437 and start timestamp401576242688 and remained active.

Next work should first establish sender/environment reliability with same-BASE,
same-seed controls and prospectively recorded dispatch/CPU/clock evidence. A
four-CPU or separated-auxiliary profile is an option to investigate, not a proven
fix. If a reliable paired control still shows a regression, isolate S1/S2/S3 and
profile the hot path before changing product code. Keep the present candidate
frozen for the user's external review; do not alter thresholds or attribute the
latency values to a specific code change without evidence. See the complete
[review prompt](REVIEW-PROMPT.md).
