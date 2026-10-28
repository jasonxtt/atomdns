# Cache-hit and read-allocation diagnostics

These tools study the three archived lossless cache/rank changes. The increment
was withdrawn on2026-10-28; use the frozen `fb61c247` source for `--shared`, since
the current restored baseline has no shared snapshot API. See the
[follow-up](../../docs/rust/validation-records/10-27-memory-lossless-cache-rank/diagnosis-2026-10-28.md).
They do not alter
retention limits or product configuration policy and never deploy a product.

## Requested allocation probe

```sh
python3 tools/memory-validation/lossless_probe.py --source /private/BASE_SOURCE --output /private/NEW_BASE
python3 tools/memory-validation/lossless_probe.py --source /private/CANDIDATE_SOURCE --output /private/NEW_CANDIDATE --shared
```

The standalone Rust diagnostic wraps System allocation and is excluded from
standard timing binaries. It checks fresh and lazy hit bytes/IDs/TTLs and the
unchanged cache original, then captures 2000 small A or 4KiB TXT entries and
exports the adapter's v2 dump. `--shared` selects the new shared snapshot API;
the baseline uses the owned API. All other inputs and assertions are identical.
Cargo dependencies are verified against the source lock; source/probe/binary
hashes and raw JSON are retained. The source Rust inventory must stay unchanged
during the diagnostic build. Destinations must be new directories.

`allocated_bytes` sums requested allocation/reallocation sizes during an operation;
`peak_requested_delta` is the peak live requested bytes above its initial live
heap. They exclude allocator metadata, rounding and residency, and do not imply
equal RSS reductions. Moka maintenance can contribute small incidental allocations.
Hit and snapshot checksums should match across same-toolchain BASE/candidate.
Dump gzip hashes need not match independent stores' randomized iteration order;
the codec unit test verifies byte equality for the same captured entry order.

## Two-CPU isolated observations

```sh
python3 tools/memory-validation/two_cpu_profile.py --source /private/SOURCE/tools --destination /opt/OWNED_UPSTREAM/tools
```

Run on Linux with CPUs0 and1 available. The destination must not exist. It creates
a private, fingerprinted copy: SUT on0; sender, CoreDNS, proxy and nice+10 sampler
on1. The standard four-CPU tools remain unchanged. This profile is not directly
comparable with observations on another host. Refuse source-marker drift or
an occupied destination. Copy this task's `lossless_post_reads.py` into the source
tools directory first when using an older archived product tree; record its hash.

Every case resets the upstream, then runs all fixture endpoint/protocol readiness
checks before starting the SUT. Original B/C,C/B,B/C order, seeds501..503,
100k fill,500QPS,60s warmup/300s measurement/30s idle and all failure/cleanup guards
remain. All protocol failures remain counted; this is observational CPU per
protocol reply, not per verified-correct query or whole-project acceptance.

After the DNS measurement/idle and upstream-counter capture, additional HTTP
reads bracket kernel HWM independently. Each rank and each named cache's show,
page and dump is read three times. Raw responses stay private. Every status must
be200; rank shape, show prefix and gzip framing are checked. These checks do not
certify complete content equivalence across independent wall-clock states.
Read peaks are separate from DNS CPU/latency and from startup/hot-apply peaks.
Missing or failed reads stop remaining slots; all prior evidence remains.

The generated `two-cpu-method.json`, per-run tool hashes, binary/build manifests,
fixture/input identities, request/reply/resource files and per-read raw bodies
are necessary for reproduction. A matching `method_head` label alone is not
proof of which derived source ran.

For a prospectively frozen second profile, `--prioritize-sender` changes only
the owned timed sender's nice value to -5. The sampler's inherited nice value
is corrected to absolute +10; SUT/CoreDNS/proxy remain at0. Source and destination
must still be distinct and new. Verify actual priorities with a short protocol
smoke before a fresh batch. This option never ignores skipped sends or repairs
an earlier incomplete batch. Low average CPU alone does not prove the cause of
a scheduling gap; record the failed batch and the new profile separately.

`performance.py --collect-incomplete-sends` is a separate, explicit collection
mode. It can finish later roles after an unsent-only shortfall with every sent
query replied, full raw rows, and no orphan/other status. All input/process/read
guards still stop the batch. Default behavior still stops at any shortfall.
The original summary continues to reject packet shortfalls; a separate
`unqualified_descriptive_metrics` field may report numerical observations.
Neither collection completion nor those descriptive numbers waive coverage,
the 5% comparison, correctness, or production acceptance. Keep prior stopped
batches. Do not replace their data or call this mode a qualified rerun.

## Offline latency decomposition

```sh
python3 tools/memory-validation/timing_attribution.py --load /private/ROLE/load --output /private/NEW_ATTRIBUTION.json
```

The output preserves statuses and missing slots, and separately reports dispatch
lag, socket round trip and original planned-to-receive latency. Socket RTT still
contains receiver scheduling. Component percentiles are not additive. The output
always records `timing_accepted=false`; it cannot qualify a failed measurement.
Before attributing a5% change to product code, require a prospectively frozen
identical-artifact/same-seed control. Control instability stops candidate timing;
changing a sender or removing compression requires a separate method and controls.
