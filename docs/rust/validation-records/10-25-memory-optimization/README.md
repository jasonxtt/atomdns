# Memory optimization: accepted stage and pre-merge validation

Updated 2026-10-26. The user accepted this stage, including CPU/protocol-reply
+5.57%, and authorized the R4 consolidation. No merge, push, release or deployment
is authorized. The historical CPU 5% gate failed; user acceptance does not turn
it into an automated PASS. Full correctness/state qualification remains open.

## Implementation and identity

Frozen candidate Rust: `b16e850dc29fd3d258ed46f557bc44c7879b34fd`;
BASE: `90997571dd29d5bd7162765e5dbc20ff16261e07`.

| Product commit | Change |
| --- | --- |
| `3c065d29` | Pool exact matcher keys and ResultIds. |
| `d7141d59` | Release payloads superseded by normalized aliases. |
| `d6e4b835` | Remove duplicate audit timing storage and borrow read allocations losslessly. |
| `a5ceb610` | Share a compact temporary ancestor-edge table during index construction. |

No config/schema/API, retention or response-payload reduction; no tiered logging
or FST implementation shipped. R1-R4 do not change the frozen product Rust.

## Correctness and qualification limits

- Constructed-provider probes: 51,435 query/variant result metadata projections
  are byte-identical for BASE/pooled/FST. This is not a full production census.
- Requested live index heap (three repetitions): 30,244,032 -> 8,106,547 B pooled;
  experimental FST 6,137,742 B. The full-inclusive production 50% goal is not closed.
- Synthetic 100k audit shared/TTL/unique heap reductions: 19.21%/11.76%/9.43%;
  duplicate timing queue removed: 4,194,304 B. Production 15% goal remains unproven.
- Final product Linux workspace: 1529 passed, 0 failed; strict Clippy passed.
  These are earlier checks, not R4 reruns. macOS workspace platform/baseline
  failures remain historical evidence; no cross-platform full PASS is asserted.
- Static-oracle aging and the K1 pilot did not qualify a state-aware oracle:
  BASE/control differed 28 times in final_upstream despite DNS/learner agreement.
- Fixed CoreDNS infrastructure passed 40 endpoint/protocol combinations and
  91,872 fixed-content queries. A multi-provider control retained 40 supplier
  differences (20 final_upstream): startup randomly selects two of three peers.
  A single-per-group bounded 272-query control passed; full qualification stopped
  on the baseline underscore-owner/CNAME defect shared by BASE and candidate.
- New documentation-address fixtures require fresh qualification for any new
  full correctness claim. Actual zone/protocol readiness, not a fixed sleep,
  is required: an initial preflight failed and cold diagnostics recorded 29
  ICMP port-unreachable/startup failures before readiness was fixed.
- The known CNAME failures, natural learner expiry and flag-publication
  differences below remain visible. Resource gates do not certify every answer.
- The two-hour soak was not run. The user accepted later production observation
  instead; that decision does not authorize production access or deployment here.

## Sustained performance and startup

Three single-CoreDNS pairs, method `7d687726`, ordinary release binaries,
60 s warmup / 300 s load. Role medians:

| Metric | BASE | Candidate | Change |
| --- | ---: | ---: | ---: |
| RSS KiB | 78912 | 60688 | -23.09% |
| CPU us/protocol reply | 157.93 | 166.73 | +5.57%, user accepted |
| p95 ms | 0.713598 | 0.710225 | -0.47% |
| p99 ms | 3.103478 | 3.096411 | -0.23% |

All 900,000 protocol replies retained, including ten SERVFAIL per role; these
are not CPU-per-correct-query measurements. Third-pair three AAAA differences
followed natural 24h no-V6 expiry; the original BASE after-expiry replay matched
candidate. Learner timestamps/TTLs were never altered.

Startup candidate HWM over six rotated rounds, KiB:
87816/87976/87688/87828/87716/87588. Candidate max 87976 versus BASE max 88872
(same-BASE control 88812). A 20k-rule build probe requested 9,893,620 -> 6,008,336 B
(-39.27%). These are construction/startup results, not all runtime heap goals.
Rejected preallocation, stale build artifact, disk-full retry and sampler/timer
interference diagnoses remain in history. A fixed-seed diagnostic retest had
CPU +6.78%, p95 +7.87%, p99 +11.02%, RSS -23.17%; no causal hotspot was established.
Earlier real-upstream observations (single-ring RSS -31.35%; three-pair RSS
-23.44%, CPU +0.06%, p95 +0.78%, p99 -4.39%) were not correctness-qualified.

## Hot-application protocol

Original method `0fc4711e`; continuation `82b6f03f`. Three alternating pairs
B/C,C/B,B/C, seeds 501/502/503; 500 qps, 75,000 scheduled questions, 150 s load,
300 s idle after sender drain. Snapshot/reset HWM at 35 s, RSS snapshot at 145 s;
peak is the larger kernel value at 145 s and sender exit, covering final load
and drain. Six operations at 40/50/60/70/80/90 s; each latency window is 15 s.
M0 uses matching windows. M1 settings and M3 dedicated-group changes restore
original values; MF toggles prefer_v4 without requery and restores the flag.

Read-only frozen config, documentation-address zones, first supplier per group,
loopback-only namespace with no default route; zone-checked readiness for every
supplier. SUT CPU 0, sender CPU 1, CoreDNS/proxy CPU 2, sampler CPU 3. M1 preserves
other overrides; M3 uses a dedicated-port-only group.

User-approved exceptions: all questions stay; SERVFAIL at frozen indices
3972/3973/5187 are counted separately, never as correct answers. The continuation
alone opts into ignoring unsent slots. Timeouts, send errors, unexpected protocol
errors, drops, operation failures and memory gates remain strict.

| Scenario | BASE median HWM MiB | Candidate median HWM MiB | Change | All-run max MiB | Max idle/M0 | Verdict |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| M0 | 68.227 | 52.285 | -23.37% | 68.500 | -- | PASS |
| M1 | 138.039 | 114.281 | -17.21% | 138.789 | 1.0636 | PASS |
| M3 | 138.168 | 113.293 | -18.00% | 139.430 | 1.0707 | FAIL: seven unsent; memory numbers within limits |
| MF continuation | 137.578 | 112.660 | -18.11% | 139.953 | 1.0527 | PASS under approved exceptions |

Candidate median <=BASE+1%; every hot HWM <=256 MiB; idle RSS <=matching
pair/role M0 x1.2. MiB = 1024 KiB. Ratios below are RSS145/idle versus M0;
P99 is the maximum of six operation windows. Every formal slot is included.

| Scenario | Pair | Role | HWM MiB | RSS145 MiB | Idle RSS MiB | RSS/M0,145/idle | Known SF | Unsent/timeout | Window max P99 ms |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| M0 | 1 | base | 66.637 | 65.520 | 66.527 | -- | 1 | 0/0 | 3.462 |
| M0 | 1 | candidate | 52.250 | 51.320 | 52.250 | -- | 1 | 0/0 | 3.448 |
| M0 | 2 | candidate | 52.285 | 51.348 | 52.285 | -- | 2 | 0/0 | 3.830 |
| M0 | 2 | base | 68.500 | 67.496 | 68.500 | -- | 2 | 0/0 | 3.472 |
| M0 | 3 | base | 68.227 | 67.262 | 68.227 | -- | 0 | 0/0 | 3.854 |
| M0 | 3 | candidate | 52.988 | 52.039 | 52.988 | -- | 0 | 0/0 | 3.681 |
| M1 | 1 | base | 138.789 | 69.941 | 70.758 | 1.0675/1.0636 | 1 | 0/0 | 4.618 |
| M1 | 1 | candidate | 114.438 | 49.480 | 50.527 | 0.9641/0.9670 | 1 | 0/0 | 4.067 |
| M1 | 2 | candidate | 111.918 | 50.031 | 51.023 | 0.9744/0.9759 | 2 | 0/0 | 4.237 |
| M1 | 2 | base | 137.223 | 69.953 | 70.762 | 1.0364/1.0330 | 2 | 0/0 | 4.109 |
| M1 | 3 | base | 138.039 | 69.930 | 70.590 | 1.0397/1.0346 | 0 | 0/0 | 5.251 |
| M1 | 3 | candidate | 114.281 | 50.066 | 51.051 | 0.9621/0.9634 | 0 | 0/0 | 4.391 |
| M3 | 1 | base | 138.168 | 70.512 | 71.230 | 1.0762/1.0707 | 1 | 0/0 | 4.730 |
| M3 | 1 | candidate | 112.645 | 49.641 | 50.684 | 0.9673/0.9700 | 1 | 0/0 | 3.865 |
| M3 | 2 | candidate | 113.293 | 50.129 | 51.129 | 0.9763/0.9779 | 2 | 0/0 | 4.248 |
| M3 | 2 | base | 137.375 | 70.629 | 71.406 | 1.0464/1.0424 | 2 | 0/0 | 4.355 |
| M3 | 3 | base | 139.430 | 71.527 | 72.660 | 1.0634/1.0650 | 0 | 0/0 | 4.722 |
| M3 | 3 | candidate | 116.672 | 49.438 | 50.395 | 0.9500/0.9511 | 0 | 7/0 | 10.779 |
| MF | 1 | base | 137.578 | 69.395 | 70.031 | 1.0591/1.0527 | 1 | 3/0 | 4.256 |
| MF | 1 | candidate | 112.398 | 49.398 | 50.449 | 0.9626/0.9655 | 1 | 0/0 | 4.076 |
| MF | 2 | candidate | 112.660 | 49.492 | 50.469 | 0.9639/0.9653 | 0 | 0/0 | 3.911 |
| MF | 2 | base | 136.492 | 70.293 | 70.922 | 1.0414/1.0354 | 0 | 0/0 | 4.298 |
| MF | 3 | base | 139.953 | 69.754 | 70.168 | 1.0371/1.0285 | 0 | 0/0 | 4.538 |
| MF | 3 | candidate | 113.492 | 49.203 | 50.160 | 0.9455/0.9466 | 0 | 0/0 | 3.777 |

Eighteen runs planned 1,350,000 queries, sent/received 1,349,993 protocol replies,
including 18 known SERVFAIL. The failed candidate returned 74,993 NOERROR and
skipped 7 sends. All other protocol errors, orphan packets, namespace UDP errors
and port 40053 socket drops
were zero. All scheduled management operations and setting-restoration checks
passed, including the failed final slot. Complete DNS/routing/state qualification
is not inferred from these resource observations.

### Original stop and retained state differences

M3 pair 3 candidate: sequence 27186..27192, planned offsets 54.372..54.384 s.
Adjacent actual sends have a 35.490 ms gap; the next sent request is 19.517 ms late.
The unchanged sender skips slots more than 20 ms behind schedule, leaving 7
`unsent`. They were not relabeled as timeouts or removed. The cause of this
dispatch gap is unproven; neither a product regression nor a specific system
disturbance is established. No rerun replaced this batch. MF/R3 subsequently ran in a separate authorized continuation.

## Paired DNS differences and natural state expiry

All 75,000 semantic projections match in each M0/M1 pair and M3 pair 1.
M3 pair 2 retains four AAAA differences involving two frozen corpus indices:

| Index | Count | Candidate ran first | BASE ran later | Frozen no-V6 expiry, Asia/Taipei |
| --- | ---: | --- | --- | --- |
| 7595 | 1 | NOERROR empty, about10:23:37 | NOERROR,1AAAA, about10:31:31 | 2026-10-26 10:31:08 |
| 1345 | 3 | NOERROR empty, about10:24:46..10:24:52 | NOERROR,3AAAA, about10:32:40..10:32:46 | 2026-10-26 10:31:36 |

Original last values 1792895468/1792895496, TTL 86400; seven-day real-IP entries
remain valid. BASE's exit snapshot has neither no-V6 entry; candidate's second
entry remains at exit, while its first expires near the end of its idle period.
This evidence is consistent with the established TTL behavior; it is not a
demonstrated candidate-only regression. All four differences remain recorded.
Query wall times are post-hoc estimates from summary-file mtime and original
monotonic timestamps, not contemporaneously sampled precise wall clocks.

Post-stop read-only comparison of M3 pair 3: all 74,993 common received projections
match; the 7 missing candidate responses are exactly the unsent sequence numbers.
This complement does not modify the original failed matrix report.

MF scheduled 450,000 queries, received 449,997 protocol replies: 449,995 NOERROR and 2 known SERVFAIL. BASE pair 1 retained 3 unsent slots under the explicit waiver. All other slots had zero unsent; all slots had zero timeout, send error, unexpected protocol error, orphan, namespace UDP error or SUT-port socket drop. No complete DNS correctness oracle or CPU-per-correct-query acceptance is asserted.

### Flag-publication differences

All differing responses are retained. Post-run comparison uses the existing question/rcode/three-RR-section semantic projection; TTL/header flags and complete wire remain separately available in the private raw data.

| Pair | Common received | Missing BASE / candidate | Differences | Planned within paired API transaction brackets | AAAA NOERROR, empty versus positive |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 74997 | 3/0 | 44 | 44 | 44 |
| 2 | 75000 | 0/0 | 34 | 34 | 34 |
| 3 | 75000 | 0/0 | 64 | 64 | 64 |

All 142 differences fall inside brackets spanning the corresponding BASE/candidate flag API transactions. Every case is an AAAA NOERROR empty-versus-positive answer; 135 positive sets contain only AAAA records and 7 include other RR types. This is consistent with queries observing different committed flag values while compilation/publication takes different times. Transaction timestamps bracket publication; they are not an exact per-query generation oracle. These responses are not relabeled equal or certified correct. There are no observed semantic differences outside those brackets.

## R3 audit operations

Each role starts with capacity 100000 and a verified full 100000-row ring. It reads stats, windows and each of five ranks ten times, clears and verifies zero stats, refills 100000, then changes capacity 100000 -> 400000 -> 100000 with readback and empty-ring checks. All 12 prefills received 100000 question/ID-matched NOERROR responses and confirmed 100000 retained rows. The existing fill tool repeats the first 128 production-corpus questions; this is not a 100000-unique-name population. Capacity 400000 was tested as a resize target, not a full 400000-row population.

Each individual API operation resets kernel HWM immediately beforehand and records before/after RSS/HWM. Read batches report the maximum of ten individual operation peaks per role/run; the gate compares the three role medians of those maxima. Clear/resize include their functional verification reads. R3 uses the unchanged candidate-median <=BASE+1% gate; no new R3 fixed cap is introduced.

| Operation | BASE peaks 1/2/3 MiB | Candidate peaks 1/2/3 MiB | BASE median MiB | Candidate median MiB | Change | Gate |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| read-stats | 58.742/58.586/58.500 | 40.484/40.145/40.340 | 58.586 | 40.340 | -31.14% | PASS |
| read-stats/windows | 61.684/61.527/61.523 | 43.602/43.195/43.512 | 61.527 | 43.512 | -29.28% | PASS |
| read-rank/domain | 65.109/64.953/64.926 | 45.867/45.461/45.840 | 64.953 | 45.840 | -29.43% | PASS |
| read-rank/client | 65.125/64.969/64.941 | 45.992/45.586/45.965 | 64.969 | 45.965 | -29.25% | PASS |
| read-rank/domain_set | 66.250/66.094/66.066 | 45.867/45.461/45.840 | 66.094 | 45.840 | -30.64% | PASS |
| read-rank/effective | 66.223/66.066/66.039 | 45.867/45.461/45.840 | 66.066 | 45.840 | -30.62% | PASS |
| read-rank/slowest | 64.090/63.934/63.848 | 44.723/44.297/44.707 | 63.934 | 44.707 | -30.07% | PASS |
| clear | 63.117/62.961/62.875 | 44.043/43.621/44.027 | 62.961 | 44.027 | -30.07% | PASS |
| capacity400k | 62.023/61.191/61.762 | 43.859/43.621/44.027 | 61.762 | 43.859 | -28.99% | PASS |
| capacity100k | 61.902/61.070/61.570 | 43.859/43.621/44.027 | 61.570 | 43.859 | -28.77% | PASS |

| Operation | BASE RSS after 1/2/3 MiB | Candidate RSS after 1/2/3 MiB | Function checks |
| --- | ---: | ---: | --- |
| read-stats | 58.742/58.586/58.500 | 40.484/40.145/40.340 | PASS in all 6 runs |
| read-stats/windows | 58.805/58.648/58.562 | 43.602/43.195/43.512 | PASS in all 6 runs |
| read-rank/domain | 62.992/62.836/62.750 | 43.680/43.273/43.648 | PASS in all 6 runs |
| read-rank/client | 62.992/62.836/62.750 | 43.680/43.273/43.648 | PASS in all 6 runs |
| read-rank/domain_set | 64.090/63.934/63.848 | 43.680/43.273/43.648 | PASS in all 6 runs |
| read-rank/effective | 64.090/63.934/63.848 | 43.680/43.273/43.648 | PASS in all 6 runs |
| read-rank/slowest | 63.117/62.961/62.875 | 44.043/43.621/44.027 | PASS in all 6 runs |
| clear | 63.117/62.961/62.875 | 44.043/43.621/44.027 | PASS in all 6 runs |
| capacity400k | 62.023/61.191/61.633 | 43.859/43.621/44.027 | PASS in all 6 runs |
| capacity100k | 60.695/60.543/58.578 | 43.859/43.621/44.027 | PASS in all 6 runs |

The last RSS in each read batch is shown; all individual before/after points remain in the private report. Across six runs this covers 438 measured API operations, plus their validation reads. Every overview endpoint returned valid JSON 200; full-ring stats, clear-to-zero and both capacity readbacks were correct. All 12 runtime cleanup/identity guards passed.

## Reproduction, provenance and cleanup

See [reusable measurement tools](../../../../tools/memory-validation/README.md)
and [offline upstream tools](../../../../tools/test-upstream/README.md).
Build separate source-keyed BASE/candidate targets with
`scripts/build-rust-native.sh`; keep dependencies locked and verify input files
and binary hashes. Prepare private input with `prepare.py` and
`tools/test-upstream/prepare_case.py`; use `operation_peaks.py` for the fixed
hot-application/audit protocol and `operation_summary.py` for the gate logic.
The tool README documents init/run and selected continuation commands. Reusing
M0 requires the original identity-checked report and identical prepared inputs;
the continuation reused M0 elapsed-time controls, not fresh wall-time controls.
Private original config/corpus/state and Linux privileges are needed; the public
sample fixtures do not reproduce the production workload by themselves.

| Binary | SHA-256 |
| --- | --- |
| Sustained BASE | `d01237abdaf9ac65d7a8457860b7590afe71289eae1b102ca9819c0da991afd7` |
| Sustained candidate | `ac1d3ce0331cb5236c7fbbc3c8db6a41d885cc4a5462c4d750c476726fb77424` |
| R2/R3 BASE | `8aedcd7cd83154457540b9d17aafa92dca1ffed4014637fed40884e7aef9e000` |
| R2/R3 candidate | `968f96bbad5e9a64bcd40560e1699edf6603929beda7ff7b1ae6de4522885b77` |

R2/R3 builds independently matched all Git build inputs (245/247 files); fresh
offline Linux builds used distinct version labels, so hashes differ from the
earlier performance binaries although product Rust is frozen.

| Evidence | SHA-256 |
| --- | --- |
| Original failed matrix report | `1a598c62d0065312ee7f0959ed8fbf4210b5cde44445f534573827af6000a331` |
| Follow-up report | `72e2be3f332e91e73a4c9baa4258ae77a419b9f553f2f0aec142d14b55b9db30` |
| Flag-window analysis | `8fc7002ec8b4f202275e5e825d9f3d7589a32022c9852a76930e7373c9e44b3a` |
| Post-stop analysis | `6a3a66439606f3e259d5121fd2f565bcea23a3c9ee8fb3ea8bae48a30160db1f` |
| Prepared inputs | `a01e303574735033aa1256b07705cb2b6deba39c9942e709c4796d93624a0e8e` |
| Full corpus | `d49e5dff5c90534ddb4fa374288b9e54978ea3a32d63dcd9b47dd5a5fd2efc7a` |
| Private preflight archive | `ef216fbd0b5366be3a4801e1f071e182ccf326408dffa69960470ca3e102add7` |
| Private diagnosis archive | `da642405fc42b26c4d9f899109b12577f9e5852b8bd808c522acc37d6752b027` |
| Private original peaks archive | `4c196812acd3be675b27d11aaa74e208d210220f976f1980a2f92882c593c864` |
| Private follow-up archive | `5b56a00b1f9688d43b187ec3f8e91a9992a38dc69c9fff6454a2903d970d6cee` |

Private archives outside Git, mode 0600, retain names/config, full wire, raw
samples, manifests and exact tools. Original peaks: 2874 members, 21 cleanup and
identity guards; follow-up: 1772 members, 12 guards. All passed. No owned runtime
processes/namespaces remain; all newly created remote runtime/tool/input roots
were removed. Original private inputs, upstream installation and existing active
service identities were preserved. No production login occurred.

R1 replaced public fixture addresses with documentation ranges, exercised fresh
subnet creation/teardown and scanned complete final branch file contents for IPs
and private paths. R4 consolidates public evidence here and removes one-off
experiments/prompts/raw JSON; it does not erase failed results. Full prior public
records, including acceptance text and experiments, are recoverable with
`git show 359ba908:<repository-relative-path>`. The branch history is retained.
Linux tools passed memory-validation 40/40 and test-upstream 15/15 before the
continuation method freeze. R4 reran both suites on Linux: 40/40 and 15/15,
without skips; local runs each skipped one environment-dependent test. Python
compilation passed after removing macOS transport metadata from the owned Linux
copy. All 39 relative document links resolve. Final branch scan: 59 files,
81 IPv4 occurrences, five IPv6, two mapped documentation IPv4, zero violations.
The four original dirty files remain unchanged; product Rust matches `b16e850d`.
The temporary R4 check directory was removed. No new product test run, review
verdict or whole-project acceptance is claimed.
