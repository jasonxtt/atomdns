# Fixed-response test upstream

Pinned CoreDNS1.15.0 provides reusable offline DNS fixtures on `mos-test`.
This infrastructure does not deploy atomdns or change production. Run tools as root
on the dedicated test host. Raw production names, zones, CA/private keys and
per-query evidence stay under private `/opt/atomdns-test-upstream`.

## Components

- `install.py`: verifies pinned official archiveSHA256, extracts only the binary,
  checks version/plugins and creates a dedicated CA/leaf. It refuses unrelated
  existing binaries. Certificate renewal is explicit: stop, archive the old cert
  folder privately, regenerate, record new fingerprints and requalify.
- `fixture.py`: generates frozen Corefile/zone files and per-supplier endpoint
  manifest. `--source` reads a complete frozen atomdns root without editing it;
  `--groups`+`--names` accepts the included synthetic sample format instead.
- `service.py`: installs the disabled-by-default systemd unit and manages only
  owned bridge/veth/namespace names. `start`, `stop`, `reset`, `status`, `check`.
- `check.py`: actual UDP/TCP/DoT/DoH/DoQ, CNAME/complex/negative/large-answer
  checks; `--full` also verifies every generated A/AAAA against the frozen zone.
- `prepare_case.py`, `case.py`, `proxy.py`: retain config rules, capacities,
  provider identities, proxy flags and group concurrency in a disposable atomdns
  namespace attached only to the test bridge. The SOCKS proxy only connects to
  declared fixture endpoints, on CPU2, in the SUT namespace so delays apply.
- `functional.py`: BASE/control ordered DNS+route+learner checks through the
  existing conservative state comparator. It never authorizes timing by itself.
  `--limit` is supplementary and explicitly marked; omit it for the full corpus.

Dependencies: `iproute2`, `openssl`, Python3/PyYAML, plus Debian client packages
`python3-dnspython python3-httpx python3-h2 python3-aioquic knot-dnsutils` for
protocol validation. The third-party CoreDNS binary needs no Go toolchain.
The original tools/e2e-1,5d-1,5d-2,memory-validation are required alongside this
directory for production-copy/state verification. No new Rust dependency.

## Initial provisioning

Copy the tools directories to `/opt/atomdns-test-upstream/tools` (excluding bytecode,
Finder metadata and private data). From the installation root:

```sh
python3 tools/test-upstream/install.py --certificates
python3 tools/test-upstream/fixture.py --groups tools/test-upstream/sample-groups.json --names tools/test-upstream/sample-names.json --output /opt/atomdns-test-upstream/fixtures/sample-v1
```

Write the literal fixture ID `sample-v1` to `/opt/atomdns-test-upstream/active`;
then:

```sh
python3 tools/test-upstream/service.py install-unit
python3 tools/test-upstream/service.py start
python3 tools/test-upstream/service.py check
python3 tools/test-upstream/service.py status
python3 tools/test-upstream/check.py --output /opt/atomdns-test-upstream/results/NEW-selfcheck --full
python3 tools/test-upstream/service.py reset
python3 tools/test-upstream/service.py stop
```

Only one test case may own `case.lock`. Output paths must be new. Stop before
changing the active fixture ID. Changed fixtures require new identities and
qualification; do not edit a running zone or overwrite old evidence. The original
private deployment currently uses `v2`, not the public synthetic sample.

## Network and timing contract

The service owns `rtup-br` (`198.51.100.1/24`), `rtup-host`/`rtup-peer` and
namespace `atomdns-upstream` (`198.51.100.2`). No default route exists there. Test
cases attach a separate veth/namespace at `.3`; they also have no default route.
Do not add default routes or change host DNS/proxy/eth0 qdiscs. Cleanup refuses
unowned interface aliases and a bridge still used by another client.

Public fixtures now use documentation addresses throughout, including generated
A/AAAA records. The bridge subnet is reserved for transport; answer addresses
use the other documentation ranges. Existing private zones, certificates and
measurements retain their original identities: generate a new fixture and new
certificates in a new root, and requalify it. Do not mix old manifests or a
running old service with these scripts. Historical published route snapshots
are sanitized copies; their retained raw-file hashes identify the private originals.

Each supplier has a distinct base port20000+10×index: UDP/TCP=base,
DoT=base+1, DoH=base+2, DoQ=base+3. TLS hostname is `localhost`; use the private
CA and pinned dial IP, never disable verification. All modes use the same zone.

Rank0 gets no artificial delay, rank1 gets30ms, rank2+ gets80ms on the upstream
veth's response path. Linux netem delays packets, including transport handshakes;
this is not exact per-query server processing time. No jitter/loss/random answer
order/cache/recursive forwarding/automatic reload is configured. CPU affinity is
SUT0, sender1, CoreDNS/proxy2; orchestration/independent sampling can useCPU3.
Ensure measured upstream CPU, qdisc drops/backlogs and socket counters show
sufficient capacity. Shared-host jitter/frequency remains a qualification concern.

**Current CoreDNS limitation:** without an EDNS buffer advertisement, its DoQ
listener truncates the large32-TXT probe. The full DoQ large-answer self-check
explicitly advertises65535 and verifies all32 records. Do not report this as proof
of full-size no-EDNS DoQ behavior; retain the observed limitation. UDP truncation
and TCP retry are tested separately. Current production-copy upstream protocols
are UDP/DoH. Malformed-wire fault cases remain in the existing dedicated mock.

## Important qualification blocker

The production-copy domestic group has3 enabled suppliers and concurrency2.
`ForwardCatalog` uses a random startup rotation seed to choose the participating
subset. Delays cannot make the fastest supplier win if it was not selected.
An initial272-query BASE/control probe had identical DNS/learner projections but
40 differing supplier projections (20 final_upstream differences); this is a
baseline qualification FAIL, not a candidate regression. No long timing batch
or candidate matrix is permitted from that failure. Any controlled concurrency
profile must be separately authorized, fingerprinted and labeled; never silently
change production concurrency or waive supplier identity.

## Checks

```sh
python3 -m unittest discover -s tools/test-upstream -p 'test_*.py'
python3 -m py_compile tools/test-upstream/*.py
```

Linux with the installed clients runs all tests; local Python without dnspython
skips the client projection test. Actual protocol/full-corpus and lifecycle
checks are separate from these unit tests. Successful fixture checks do not
prove full-program memory gates, performance gates or two-hour stability.


## Single-supplier controlled profile

The user subsequently selected `functional.py --single-upstream` instead of
changing concurrency. Each business group keeps its first enabled supplier,
using the same CoreDNS instance, original transport/proxy and fixed group data;
the original provider-count mode remains the default. Prepared manifests name
this profile `single-per-group`. Production is untouched.

A bounded272-query BASE/control probe passed all DNS/route/learner comparisons.
The full probe stopped on a baseline SERVFAIL; synthetic controls confirmed a
pre-existing underscore-owner+CNAME failure in both BASE and candidate. No
performance acceptance follows from the bounded pass. See the [consolidated qualification record](../../docs/rust/validation-records/10-25-memory-optimization/README.md#correctness-and-qualification-limits). The lifecycle tool also has a regression for veth removal
racing namespace cleanup; missing devices alone are benign, persistent errors
and ownership changes are not waived.

## Three paired controlled performance observations

After the user explicitly chose to defer the known CNAME defect and continue
performance comparison, `performance.py` runs a separate observation path. It
keeps every production question/error, uses `single-per-group`,100k prefill,
500QPS,60s warmup/300s measured/30s idle, B/C,C/B,B/C and seeds501..503. CoreDNS
is reset for each case. SUT0/sender1/upstream+proxy2/independent nice+10 sampler3.
The sender records full DNS wire as well as decoded replies. Per-sequence DNS
semantic differences and every rcode remain visible; no static multi-answer
oracle, question filtering or product fix is introduced.

```sh
python3 tools/test-upstream/performance.py --source PRIVATE_FROZEN_ROOT --base BASE_BINARY --candidate CANDIDATE_BINARY --output NEW_PRIVATE_OUTPUT --method-head FULL_METHOD_COMMIT
```

The full functional/learner guard remains failed/incomplete, so these observations
must not be called CPU per correct query or whole-project acceptance. The driver
rejects identity changes, non-full ring, missing packets, unclean cases or changed
protected config; it stops remaining slots on an incomplete observation. Generated
special_groups.yaml must prove parsed equality, while learner/top-domain state
changes retain original before/after hashes. No rerun-until-green or threshold
change. `performance_summary.py` reports all pairs, role medians and all-run max
HWM separately. Resource and serialization overhead is equal for both roles;
upstream/proxy/sender/sampler resource traces are retained for attribution.
