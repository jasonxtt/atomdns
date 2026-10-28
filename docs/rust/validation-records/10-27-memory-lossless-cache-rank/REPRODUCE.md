# Reproduction

Original private root on `ssh mosdns-rust` is
`/root/rtdns-lossless-20261027`. Preserve it; every output below must be a new
directory. Frozen inputs, binaries, manifests, all derived tools and protocol
dependencies are retained. Python3.13, Rust1.95.0 and CoreDNS1.15.0 were used.
Original dataset/learner timestamps remain untouched; a later replay can age
that state differently. Do not reset timestamps to force an equality result.

## Ordinary binaries

From the archived `base/` or `candidate/` source directory, run the standard
script with a new output path and the role's exact source/version labels:

```sh
BUILD_VERSION=v0.4.0-lossless-base-3687d269 \
BUILD_SOURCE_ID=3687d269b1699b7061a3b48c5e4a9f51745bc625 \
OUTPUT=/root/rtdns-lossless-20261027/binaries/NEW_BASE \
CARGO_TARGET_DIR=/root/rtdns-lossless-20261027/NEW_BUILD_TARGET \
scripts/build-rust-native.sh
```

Candidate labels are `v0.4.0-lossless-candidate-fb61c247` and
`fb61c2474e663e39c88e1aa55831eb8006731bfb`. Verify the produced manifest and
source inputs; original artifact hashes are recorded in `evidence.json`.

Linux checks used the candidate Rust workspace:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
CARGO_TARGET_DIR=/root/rtdns-lossless-20261027/NEW_TEST_TARGET \
cargo test --manifest-path /root/rtdns-lossless-20261027/candidate/rust/Cargo.toml --workspace
cargo fmt --manifest-path /root/rtdns-lossless-20261027/candidate/rust/Cargo.toml --check
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
CARGO_TARGET_DIR=/root/rtdns-lossless-20261027/NEW_TEST_TARGET \
cargo clippy --manifest-path /root/rtdns-lossless-20261027/candidate/rust/Cargo.toml --workspace --all-targets -- -D warnings
```

The test profile overrides only reduce debug artifact disk use. Ordinary release
timing binaries were built separately by the standard script above.

## Controlled fixture and timed collection

The installed fixture is stopped/startup-disabled after the original run. Its
service and network are owned test resources; existing product services must
remain untouched. The harness checks exclusive case ownership and fixture/input
identity, then checks all40 endpoint/protocol combinations before each SUT.

```sh
python3 /opt/rtdns-test-upstream/tools/test-upstream/service.py start --root /opt/rtdns-test-upstream
env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy \
PYTHONPATH=/root/rtdns-lossless-20261027/pydeps \
taskset -c 1 python3 /root/rtdns-lossless-20261027/profile-collection/tools/test-upstream/performance.py \
  --source /root/rtdns-lossless-20261027/private-source \
  --base /root/rtdns-lossless-20261027/binaries/base \
  --candidate /root/rtdns-lossless-20261027/binaries/candidate \
  --output /root/rtdns-lossless-20261027/NEW_COLLECTION \
  --method-head 7b72bba7f330c60aca1005cc77be0c78981a78cd \
  --collect-incomplete-sends
```

This collects later roles after an unsent-only incomplete role. It does not pass
coverage: the original summary still reports `incomplete` and numbers are under
`unqualified_descriptive_metrics`. Omit the flag for default stop-on-shortfall.
Neither mode allows a green replacement of earlier failed evidence.

## Closed-loop read matrix

This is a separate experiment with fixed full corpus,100000 replies per role,
and75 HTTP reads; it makes no DNS timing claim.

```sh
env -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY -u http_proxy -u https_proxy -u all_proxy \
PYTHONPATH=/root/rtdns-lossless-20261027/pydeps \
taskset -c 1 python3 /root/rtdns-lossless-20261027/profile-reads/tools/memory-validation/lossless_read_matrix.py \
  --source /root/rtdns-lossless-20261027/private-source \
  --base /root/rtdns-lossless-20261027/binaries/base \
  --candidate /root/rtdns-lossless-20261027/binaries/candidate \
  --output /root/rtdns-lossless-20261027/NEW_READ_MATRIX \
  --method-head cbff46d20f64cb24b15e09174269ee30db741f75
```

Original derived profiles came from the Git tools archives at their stated method
heads using `two_cpu_profile.py`. Collection adds `--prioritize-sender`; read
matrix does not. Re-derivation must use new destinations. All138 derived Python
hashes were reproduced independently and matched the used profiles.

From the repository, compare a downloaded matrix's raw bodies without printing
query names or answers:

```sh
python3 docs/rust/validation-records/10-27-memory-lossless-cache-rank/compare-read-outputs.py \
  --root /tmp/rtdns-lossless-20261027/read-matrix-v1 \
  --output /tmp/rtdns-lossless-20261027/NEW_READ_COMPARISON.json
```

Only absolute timestamps and dump entry order are normalized. Every raw-body
hash is checked first. The output refuses overwrite.

Stop the owned fixture after no case holds it:

```sh
python3 /opt/rtdns-test-upstream/tools/test-upstream/service.py stop --root /opt/rtdns-test-upstream
```

## Allocation and setup diagnostics

See [allocation/profiling method](../../../../tools/memory-validation/LOSSLESS-READS.md).
The requested-allocation probe is separate from timing executables. Root
`sender-priority-smoke/`, `raw-gzip-diagnostic/`, `sender-diagnostic-v1/`, their
controllers and source hashes retain setup/diagnostic evidence. These do not
qualify performance. Raw config, answers, credentials and certificate keys must
remain private; public evidence contains identities and numerical results only.
