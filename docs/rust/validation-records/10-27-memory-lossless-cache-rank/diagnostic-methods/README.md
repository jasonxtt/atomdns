# Frozen diagnostic sources

These task-specific sources match the executed SHA256 identities in
`../diagnosis-evidence-2026-10-28.json`. They are not product code or acceptance
tools. Their private paths refer to the retained mosdns-rust setup described in
`../REPRODUCE.md`; outputs must be new and existing services must remain untouched.

- `run-timing-cause-controls.py`: exact BASE/BASE first, then planned C/B/C/B,
  same seed501. Each role uses the frozen original two-CPU profile and Case guards.
  The owner-directed supervisor stopped after the two BASE roles exceeded the
  prospectively documented absolute5% reliability threshold. The candidate
  startup check was interrupted before SUT creation, so four planned roles did
  not run. The script is retained unchanged, including its original longer plan;
  the control decision and partial-run receipts are part of the evidence.
- `sender-direct-scheduling.py`: instruments the original Python sender and
  explicitly changes its target to the fixture's198.51.100.2 address. Scheduling
  wait, CPU, context switches and GC are diagnostics, not uninstrumented timings.
- `run-direct-scheduling.py`: run inside the owned fixture namespace with the
  private dependency path; first checks20 UDP answers, then runs360s direct
  CoreDNS traffic. No rtdns process or management action is on that DNS path.
- `hit-timing.rs`: separate non-instrumented API microbenchmark, compiled against
  each archived product tree using the retained `hit-timing-*-linux/Cargo.toml`
  and frozen lockfiles. It is not the release executable. Fresh/lazy stages use
  a fixed test clock, and each stage performs2000000 lookups. Linux process
  affinity was0; pairs ran B/C,C/B,B/C. All individual values, including the
  negative lazy-small result, remain published. The simple checksum is a sanity
  check only; exact correctness comes from the separate functional tests.

The source controls were intentionally not repeated until a green result. The
subsequent withdrawal restores the pre-increment Rust tree; it does not repair
or qualify this original measurement method.
