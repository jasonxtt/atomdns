# Performance attribution follow-up — 2026-10-28

The owner requested root-cause investigation and exclusion of the increment if
its whole-program safety could not be established. Earlier failed observations
remain immutable. No exception to coverage or the 5% margin is authorized.

## Identical-source control

Both controls used the exact original BASE artifact
`ada3564b5bf0430e3332784f0b807dced6619d62faaca920ab736c30d018917b`
from `3687d269`, the same frozen production-derived configuration/corpus and
seed501. Each fresh isolated process received100000 prefill requests,60s warmup
and300s measurement at500QPS. Original two-CPU sender/profile sources stayed
unchanged. Builds and other benchmarks did not run concurrently. The second
control's stopping condition was fixed before its measured window: any missing
send or absolute CPU/p95/p99 change above5% stops the remaining candidate slots.

| Metric | BASE control A | Same BASE control B | B/A change |
| --- | ---: | ---: | ---: |
| Measurement replies | 150000 | 150000 | 0 |
| Unsent / orphan | 0 / 0 | 0 / 0 | — |
| CPU / protocol reply, μs | 151.466667 | 163.466667 | +7.9225% |
| Planned-to-receive p95, ms | 0.663226 | 0.696814 | +5.0643% |
| Planned-to-receive p99, ms | 2.600626 | 3.029525 | +16.4922% |
| Median RSS, KiB | 63840 | 63738 | -0.1598% |

All150000 common DNS semantic projections match; both contain the same7 known
SERVFAIL replies. This is not a comparison of raw TTL, flags or route-learner
state. Full rings, protected inputs, completed-case cleanup and original-service
identity checks pass. The candidate-a setup was interrupted before its SUT or
measurement started; its cleanup correctly records no created namespace.

**Finding:** the current measurement setup can exceed the regression margin with
no source or workload change. The original candidate's aggregate latency numbers
cannot independently establish a code regression. This also does not prove the
candidate safe: an unreliable control cannot produce a passing acceptance result.
This excludes source changes as the cause of the control difference, but does not
separate scheduling, machine speed, time-dependent baseline state or other runtime
variation. Their individual contributions to the CPU change remain unresolved.

## What the latency measured

The original `real_metrics.summarize` measures `finished - planned`. This includes
dispatch delay before any request reaches rtdns. `finished - sent` is a separate
socket round trip that still includes receiver scheduling and is not pure server
execution time. The new read-only `timing_attribution.py` reports both components,
preserves every failure and never changes qualification. Separate component
quantiles must not be added to reconstruct the total quantile.

Original candidate missing slots occur in six send gaps of approximately
33.687/60.529/84.046/21.967/35.175/30.207ms. The sender intentionally skips slots
over20ms late. Before three gaps, its preceding DNS reply had already completed
before the next slot was due. Actual SUT socket drops were zero. The missing
requests are sender coverage failures, not measured rtdns packet loss. The old
raw rows remain necessary; this diagnosis does not erase those failures.

## Why steady RSS did not fall

S1 removes a temporary response copy; S2 reduces grouped-rank result capacity;
S3 removes temporary cache export/list copies. They do not change live cache
payloads, audit-ring retention or rule storage. During the frozen DNS window,
rank/show/dump are not called and automatic cache save is outside the window.
S1 is the only changed query path. S2/S3 can still affect executable layout, so
call-path inspection is not a substitute for whole-program qualification.

The earlier cache-show/dump read-peak reductions remain real observations of the
archived candidate. They are not retained-heap or steady-state RSS reductions.
The rank input remains O(N), and reserving less output capacity need not reduce
resident pages proportionally. The report must keep those memory categories
separate; it must not sell this increment as compression of the running log ring.

## Supplementary hit-path diagnostic

Separate, non-instrumented lookup probes used the same existing real
`NativeCacheAdapter`, fixed fresh/lazy clocks,53B A and4145B TXT responses,
2000000 lookups per case, three B/C,C/B,B/C pairs. Both checksums match in every
case. These are API microbenchmarks, not DNS/whole-program acceptance.

Mac medians showed lower CPU in all four cases. Linux medians showed lower CPU
in fresh-small and both large cases, but higher CPU for lazy-small; independent
BASE process results also varied. All results are retained, including that
negative case. No CPU benefit or waiver is inferred from this microbenchmark.

## Direct-fixture scheduling diagnostic

An explicitly instrumented sender bypassed rtdns completely and sent the same
seed501 schedule directly to the private CoreDNS UDP endpoint. No rtdns process
was on that network path. All180000 warmup-plus-measurement replies arrived with
NOERROR, zero unsent and zero orphans. **The historical71 misses were not
reproduced.** This is a mechanism diagnostic, not an original-profile control or
candidate acceptance run; instrumentation itself adds work.

During the measured300s, planned-to-receive p99 was2.5524ms while dispatch p99
was1.6249ms and socket-roundtrip p99 was0.8639ms. These independent percentiles
are not additive. The absence of rtdns proves these numbers include substantial
measurement-side work; it does not prove identical overhead in the earlier runs.

The full360s trace records a synchronous gzip write taking7.168ms:
3.187ms thread CPU and3.912ms runnable-queue wait, with three involuntary context
switches, no major page fault and no output blocks for that span. Another write
took6.074ms with5.958ms thread CPU and zero runnable-queue wait. A decode span
took4.174ms with0.148ms CPU and3.960ms queue wait. Thus both raw compression
and scheduling demonstrably stall this sender's shared receive/send loop.
Generation2 GC did not occur in this diagnostic; that is not a finding about
every earlier gap. No GC/priority/compression or product scheduling change was
adopted as an unproven fix.

## Disposition and verification

The three product commits are reversed by `0868fded7ddfb58829a4c293c9bd37a99f40e9a7` in the research branch. Rust and the
247 build inputs exactly match `3687d269`; the older accepted domain-index/audit
changes remain. The historical `fb61c247` candidate and all failed evidence stay
available in Git/private archives. Candidate-specific contract paragraphs are
removed, while historical probe tools explicitly require archived candidate
source for shared-snapshot experiments. The old review prompt is no longer an
active submission. No source-level cause for the original whole-program
regression has been established, and no acceptance PASS is claimed.

A first withdrawal test invocation reused the earlier Cargo target directory;
it ran a candidate-only test absent from BASE source. **That invocation is
excluded regardless of its exit status.** Fresh-source verification and a new,
previously absent Cargo target were used instead: **1533 passed,0 failed,0 ignored**
in99 target summaries. Candidate-only tests are absent. Linux workspace formatting
and strict all-target Clippy pass. All247 remote source inputs still match BASE.
The baseline standard-build artifact is reused only after exact input identity
verification; no fresh release build or deployment is claimed.
This does not affect the ordinary timing artifacts, whose source-keyed standard
build identities were already verified against Git.

New offline attribution tests pass3/3 on both Mac and Linux, including preservation
of missing/failed records, clock errors, and non-additivity of percentiles.
Numerical data and raw identities are in
[the follow-up evidence](diagnosis-evidence-2026-10-28.json). Private evidence
remains under `/root/rtdns-lossless-20261027` on mosdns-rust. All owned fixture/case
namespaces are removed and the private fixture is stopped; the existing service
still has PID827437/start401576242688. No production service or main branch changed.
