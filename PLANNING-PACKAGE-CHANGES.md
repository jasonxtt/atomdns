# Planning package v2 corrections

This package supersedes the first downloadable planning draft.

1. Roadmap ownership is explicit. `10-06-rust-native-core-query-primitives` is the next child/frontier of `09-28-rust-next-step-roadmap`. Completed `domain_set` management is historical and must not be rerun. Slice 1 corrects stale parent/next-stage/handover text before product code.
2. Baseline is updated to the current clean repository HEAD `83b86cd90cc15f23b3a882226e06551b909045ef`, aligned with `origin/rust`.
3. Switch SHAs are retained as reviewed evidence ancestry rather than used as the working-tree baseline.
4. `4578882c... -> 83b86cd9...` was checked and contains only two Trellis backend spec documents, no product-source change.
5. Implementation start must refresh HEAD/status/evidence and regenerate `research/source-baseline.json` if anything moved.


## v3

- Reverses v2's parent-child decision after reviewing the old roadmap's finite closeout contract.
- `09-28-rust-next-step-roadmap` is now treated as historical finite coordination work to be closed/archive as `superseded`.
- `10-06-rust-native-core-query-primitives` is independent with `parent: null`.
- Historical unchecked roadmap workflow boxes are preserved; no retroactive fake completion.
- Added `research/09-28-roadmap-closeout-plan.md`.
- Long-lived ordering authority is explicitly moved to `next-stage-plan`, handover and feature coverage.
