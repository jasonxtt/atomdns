# Exact retained domain count (2026-10-26)

User requested the number of distinct queried domains and explicitly chose the
current retained audit records, matching the query-count coverage. Parent:
`f2fc0016`. This increment is not a production deployment or review PASS.

## Change

The overview previously counted rows returned by a domain ranking limited to500
and displayed500+. It now requests20 display rows and reads the full distinct
count from `X-Audit-Distinct-Domains` on that same response. Missing/invalid
metadata displays unavailable; a real empty ring displays0.

The backend counts canonical question names during its existing domain bucket
projection before rank truncation. A/AAAA, repeats, ASCII-case variants and a
terminal dot do not inflate the count. Ring eviction, capture eligibility,
clear and capacity changes naturally affect coverage. This is not lifetime
or cached-domain cardinality; a restart starts an empty retained ring.

Existing domain-rank JSON arrays, raw labels, counts, ordering and limits remain
unchanged. The count does not require another persistent domain set, extra
full-name copies or another full aggregation. Worker gates, cancellation and
allocation failures retain existing handling. O(1) stats and client-count scope
are unchanged. Other consumers can ignore the additive response header.

## Checks

| Check | Result |
| --- | --- |
| RED real DNS/HTTP550 distinct with limit20 | Failed: no full-count metadata |
| Final real DNS/HTTP553 records,550 domains |550 with limits20 and500 |
| A/AAAA/case/dot, raw rank array/order, capture stop/start | Passed |
| Resize, last-name eviction, clear and empty ring | Passed |
| Local affected Rust |304 unit +6 HTTP integration =310 passed |
| Linux affected Rust |312 unit +6 HTTP integration =318 passed |
| Frontend |83 passed; build passed |
| Strict workspace Clippy / formatting / diff whitespace | Passed |

The retained-audit BASE fixture remains unchanged. Its test explicitly asserts
the new header has value4 for the fixed fixture, then excludes only that approved
addition from comparison; every original response byte is still checked.
The initial fixture check correctly failed on the added header before this
allowlist was introduced. No production traffic, config or data was used.

Linux checks used a new isolated source/target with one build job. The tested
Rust inputs were hash-checked, owned processes/directories cleaned and existing
test services preserved. Raw logs remain outside Git. No new whole-workspace
or performance/heap acceptance claim; review remains pending for this range.
