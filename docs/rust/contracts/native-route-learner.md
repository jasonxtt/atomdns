# Native `route_learner` (M2-2)

Scope: the learned routing facts of the Rust host — remembered direct / proxy, remembered no-V4 / no-V6 and special names — as one store with per-entry
expiry, merged into a `domain_mapper` when it evaluates a name. It replaces the former `domain_output` learning lists (`target_mapper`,
statistics file → rule file → internal POST → mapper rebuild), which config schema 2 removed. Evidence: [10-22-route-learner](../validation-records/10-22-route-learner/README.md).
`top_domains` (the access ranking) stays a plain [`domain_output`](native-domain-output.md).

## Configuration

`route_learner` (one per mapper; unknown arguments are errors):

| Argument | Meaning |
|---|---|
| `target_mapper` | Required. A `domain_mapper` tag. One learner per mapper. |
| `file` | State file, relative to the configuration root; default `gen/route_learner.json`. |
| `write_interval` | Seconds between periodic writes, 60–31,536,000; default 21,600 (6 h). |
| `max_entries` | Default capacity of a category, 1–262,144; default 20,000. |
| `categories` | Required, non-empty list; **its order is the order of the labels in `domain_set`**. |

A category: `tag` (unique across all plugin tags and categories; it keeps the old `domain_output` tags `my_realiplist`, `my_fakeiplist`, `my_nov4list`, `my_nov6list`,
`my_sv4list`, because `/plugins/<tag>/…` and the WebUI domain-generation switches address them), `mark` (1–63, the fast mark the mapper sets), `label` (the `domain_set` string; the audit
`effective_tag` logic matches these strings, so `记忆直连`, `记忆代理`, `记忆无V4`, `记忆无V6` must not change; no `|`), `source` (shown as `matched_rule_source`; default the tag; no `|`), `ttl`
(seconds from the last confirmation, 60–315,360,000, required), `max_entries`, `enabled` (default true; the settings layer overrides it, see below).

`route_learn` (a rule, dispatched as `exec: $tag`): `learner` (a `route_learner` tag), `learn` (a category tag or a list), `when` (optional; one expression or a list in the sequence `matches` syntax, all must hold,
compiled by the same code as `matches`). The rule's tag is the *reason* stored with the entry. A rule never changes the query, its routing or its answer; it fails the query only when a `when` matcher fails
(as a sequence rule would). A learned name takes effect from the **next** query.

## Store

Per category a table keyed by the presentation-form name, ASCII lower case, without the trailing dot (empty names, the root and names over 253 bytes are not recorded). An entry has the first and last
confirmation time, a hit counter and the reason. Entries expire individually `ttl` seconds after the last confirmation; an expired entry is invisible and removed when met (and by a sweep before each write).
Confirming a live entry only refreshes its last confirmation; confirming an expired one starts it afresh. Using a learned route is **not** a confirmation: only a rule firing is, so a stale conclusion is re-decided by the
normal probing path once it has expired and the next query misses the cache. A full category first drops expired entries, then the least recently confirmed sixteenth. Nothing is imported from the former
`gen/*list.txt` / `gen/*rule.txt`; they are neither read nor modified.

## Mapper merge

The learner's categories are looked up in category order. A hit or-s `1 << mark` into the mapper's fast flags, puts the label **before** the matcher's labels (and `source` before the matcher's sources, duplicates dropped),
and counts as a match: the mapper default mark/tag are not applied even when no rule matched. Learned facts and matcher rules are a union; which route a name takes is decided by the sequences' `fast_mark` order, not here.
No matcher is rebuilt for learning.

Response-cache hits preserve this current query's learned-label prefix before
the cached non-learning labels. They must not restore a historical learning
label after expiry or flush, or restore a mapper default over a live learned
match. The existing mapper lookup supplies the prefix; cache restoration does
not perform another lookup or increment learning hit counts. Explicit later
label assignments replace the mapper context. The cached response's
`effective_tag` is not relabelled by this query-local annotation. See the
[cache lifecycle contract](native-cache-lifecycle.md).

## State file

`gen/route_learner.json`: `{"format":1,"saved_at":<unix>,"categories":{"<tag>":[[name, first, last, hits, reason], …]}}` (seconds). Written through a same-directory temporary file and a rename.
Written: every `write_interval` **when an entry changed** (new, confirmed, expired, evicted, flushed; hits alone do not count), by `save`, immediately after `flush`, and when the host stops (also hits). A hot apply writes nothing.
A failed write keeps the previous file, is logged and retried at the next tick. Read when the host starts serving: a missing file is empty; an unreadable file or unknown `format` is logged and the host starts empty; bad rows, expired entries and categories
that are not configured are skipped; a category over capacity keeps its most recently confirmed entries.

## Hot apply

A settings or special-groups apply compiles a new generation. Before the old one is fenced the new generation takes over the old learner's live state **in memory** when both have the same categories (otherwise the old state is
saved and the new one loads it). Queries still running on the old generation keep learning into the same state; only the new generation writes.

## HTTP (`/plugins/…`, GET only; other methods 405, unknown action 404)

Category tag: `show?q=&limit=&offset=` (text; a line per live entry `%010d <date of last confirmation> <name>` by **hit count** descending, then name; `limit` 100 when invalid or ≤0; header `X-Total-Count` = live matches, exposed to browsers),
`save` (writes the file when something changed; 200 `domain_output saved`), `flush` (clears that category, writes at once, 200 `domain_output flushed`), `restartall` (501). `save`/`flush` answer 503 while the generation is retired.
Learner tag: `entries?category=&q=&sort=hits|last|first&order=desc|asc&limit=&offset=` (JSON; `limit` ≤ 1000, default 100; each entry has `category`, `conclusion`, `mark`, `name`, `reason`, `first_confirmed`, `last_confirmed`, `expires_at`, `hits`),
`status` (per-category counts, capacity, ttl and enabled; counters learned/confirmed/expired/evicted/flushed since start; the last write).

The capabilities `lists.remembered` and `system.domain_generation` are supported when a `route_learner` (or a `domain_output`) exists. The WebUI domain-generation switches (`/api/v1/domain-generation`, `webinfo/settings.json`
`domain_generation.plugins.<category tag>`) set the category's `enabled`; a disabled category neither learns nor answers, and turning one off flushes it.

## Configuration schema 2

`REQUIRED_CONFIG_SCHEMA = 2`, package id `rust-config-schema-2`. A `domain_output` with `target_mapper`, `target_mark` or `target_tag` is a configuration error that names this plugin.
