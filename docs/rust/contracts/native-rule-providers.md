# Native rule providers, `domain_mapper` and `mark` (M1-a1, M1-a2)

Scope: the read side that lets the native host load the rule providers a production
`config_all` uses and route queries by `domain_mapper` marks (M1-a1), and the management side
(source add/edit/delete, download, auto-update, `show`/`save`/`post`, hot reload; M1-a2, section
"Management" below). Evidence: `docs/rust/validation-records/10-07-m1-a1-rule-providers/` and
`docs/rust/validation-records/10-07-m1-a2-rule-management/`.

## Providers

All providers load at compile/start time from existing disk state; compiling (and `check`)
never writes. Writes, downloads and reloads happen only in a running host, through the
management API and the background jobs described under "Management". Relative paths follow
the configuration root (`-d`).

| Type | Arguments | Source of rules |
| --- | --- | --- |
| `domain_set_light` | `exps`, `files`, `sets` (ignored) | `exps`, then each file: SRS or text. A missing file is an empty source. |
| `sd_set_light` | `local_config` (required), `socks5` (downloads) | JSON manifest; enabled sources with a `files` path; each file is SRS or rule text (see "Subscription formats"). `enable_regexp` gates regexp items. |
| `si_set` | `local_config` (required), `socks5` (downloads) | same manifest shape; CIDRs of enabled SRS or CIDR-text files; usable in `resp_ip $tag`. |
| `adguard_rule` | `dir` (required), `socks5` | `<dir>/config.json` + `<dir>/<id>.rules` of enabled lists (AdGuard / hosts text; other download formats are converted to it). Only block rules are exported; `@@` lines never reach the mapper. Lists that are not on disk are skipped at compile time and downloaded in the background once the host serves. |

WebUI grouping of the five 分流规则 online sets (interface only; the API and the manifests do not enforce it): 基础规则 are `geositecn`/`geositenocn`/`geoipcn` (中国域名/非中国域名/中国 IP), which the WebUI keeps at one source or more per type, cannot be added as new types, asks to confirm before one is turned off and can restore the default address of; 自定义订阅 are `cuscn`/`cusnocn` (直连域名/代理域名), freely added, edited and deleted. New subscriptions can only be of these two types.

A missing, empty or unparsable manifest is an empty source list (diagnostic only); a manifest with a string the reference repairs (lone UTF-16 surrogate escape, invalid UTF-8) is refused instead, because Go would keep its sources. Manifests decode like Go's `encoding/json` into each plugin's own struct (case-insensitive keys applied in document order, `null` = zero value, wrong-typed known fields and non-RFC 3339 `last_updated` fail the whole file; `tools/m1a1-oracle/manifest_parity.py`); an enabled
source whose file is missing, unreadable or not a valid SRS is skipped with a diagnostic.
Unknown arguments are configuration errors. `domain_set_light`/`sd_set_light` have no executable
or matcher form (their reference `Match` is constant false, so `qname $tag` is not supported for them);
`domain_mapper` consumes them, and `si_set` is consumed by `resp_ip`. `adguard_rule` additionally works as a
direct `qname $tag` matcher with the reference semantics: a name matches when no `@@` exception rule
matches and a block rule does (domain rules are case-insensitive, regexps match the normalized name);
`domain_mapper` still consumes only the block export.

## `domain_mapper`

Arguments `rules[]` (`tag`, `mark` 0..=63, `ctx_mark`, `output_tag`), `default_mark` (0..=63),
`default_ctx_mark`, `default_tag`. A rule tag must name a rule provider above; the same provider
may appear in several rules. Execution takes the query name in DNS presentation form and
applies, on a match, fast flags (bit `mark`), context marks, the joined output tags to the
routing `domain_set` field and the joined sources to `matched_rule_source`; with no match it
applies the defaults (flag, context mark, tag; nothing when zero/empty). Matching, inheritance
and merging are the `ValuedDomainMatcher` semantics, verified against the reference mapper.
Rules the reference mapper skips (no type prefix, unknown type, a regexp Go rejects) are skipped with a
diagnostic; a non-regexp rule with a non-ASCII pattern is skipped too (a query name in presentation form is
always ASCII, so such a rule cannot match anyway). Exception: a `domain:`/`keyword:` pattern that the
reference's Unicode lowercasing turns into ASCII (for example the Kelvin sign, U+212A, becomes `k`) can match
ASCII names, which this host does not reproduce, so the configuration is refused naming the rule.

**Regexp rules never disappear silently.** Patterns run on the Rust engine after a Go-compatibility gate
(`matcher-core/src/regex.rs`): the usual Go forms (`\d \w \s \b`, `(?:…)`, `(?i)`/`(?i:…)`, POSIX classes,
`\xHH`, lazy quantifiers, counted repeats up to 1000, escaped punctuation) are accepted and behave identically on
the ASCII names the matcher sees. A pattern Go rejects (nested repetition, bad counts, unbalanced brackets or
parentheses, …) is skipped like the reference does. A pattern that is **valid Go but outside the gate** (`\p{L}`,
`\Q…\E`, named groups, lookaround-looking groups, literal braces, non-ASCII, `\a`/`\v`/`\0`, …) makes the
configuration fail with a message naming the rule, on the `domain_mapper` path and on the direct
`qname $adguard` path alike. Parity evidence: `tools/m1a1-oracle/regexp_parity.py` (257 patterns × 39 names
against Go's `regexp`; no violations).

**Deviation (approved 2026-10-07):** `full:` rules match case-insensitively and rules that
differ only in case are merged; the reference matches `full:` case-sensitively on the raw
name. Everything else is identical on the production corpus.

## `mark`

Quick matcher and executor over **context** marks (`mark N [N…]`, base-10 `u32`); independent of
`fast_mark`.

## Limits and known differences

- SRS files: header must be `SRS` v1–3 + zlib; decoding is bounded in bytes (64 MiB inflated, 128 MiB
  file) and in objects (2,000,000 rules/strings/prefixes per file, 262,144 words per succinct-set
  bitmap, 16 MiB of labels, 64 MiB of materialized keys, nesting 16; declared lengths never pre-allocate). A damaged body yields the rules read before the
  damage (diagnostic); the reference reads on and may desynchronize.
- `domain_set` and `sd_set` are rule exporters in the reference host but are not accepted as `domain_mapper` providers here
  (reported as "does not support rule export"); `config_all` does not use them that way.
- The order of joined source names for one `sd_set_light` is manifest order (the reference's is random).
- Memory: the mapper's matcher is not yet compact (about 150 bytes per `domain:` rule; Mem-2). See the
  resource notes in the M1-a2 and Mem-1 validation records.

## Subscription formats

`sd_set_light`, `si_set` and `adguard_rule` sources accept more than SRS. The content of a download is converted
once, when it is downloaded; start-up and hot reloads read only the stored file.

**`format` field.** Optional in the manifest (and in the `PUT /config/{name}`, `POST|PUT /rules` bodies and their
JSON views): `auto` (default), `srs`, `text`, `yaml`, `json`, `dat`, `mrs`. `auto` is never written, so a manifest without the
field is written back byte for byte as before. The value is read without regard to case. A manifest with an
unknown or non-string value reads it as `auto` (diagnostic, the manifest stays usable); the API answers 400.

**`category` field.** Optional string in the same places as `format`, trimmed, written only when not empty. It names
the category of a `.dat` file to import (`cn`, `category-ads-all`, …) and is ignored for every other format. With
`format` `dat` it is required: the API answers 400 (`category is required for format dat`), and a manifest entry
without it fails its download. An `auto` source whose address ends in `.dat` needs it as well (the download fails
without it).

**Detection (`auto`)**, in order: the SRS magic (`SRS` and a version byte 0..=3) is `srs`; the extension of the
content (a zstd frame whose first four decoded bytes are `MRS\x01` is `mrs`); the extension of the
download address path (`.dat` is `dat`, `.mrs` is `mrs`; `.srs` is `srs` too, so a `.srs` address that returns a page fails as it always did; `.yaml` `.yml` is `yaml`, `.json` is `json`, `.txt` `.list` `.conf` is `text`); the content
(first non-blank character `{` is `json`, a line starting with `payload:` is `yaml`); otherwise `text`.

**Storage.** An SRS download is stored as it is for `sd_set_light` and `si_set`. Every other download is converted
to rule text and stored at the `files` path: a first line `# atomdns: converted from <format> <url>`, then one rule per
line (`domain:`, `full:`, `keyword:`, `regexp:` for a domain set; `address/bits` for an IP set). A domain set takes
only domain rules and an IP set only IP rules; the other kind is counted as skipped. Loading reads SRS if the file
has the SRS magic and rule text otherwise, with the same line reader, so a text file placed by hand works too.
`adguard_rule` stores AdGuard / hosts text as it is when it holds at least one usable rule (`@@` lines included) and
every line that would give a domain rule as rule text is also read by the AdGuard reader; anything else (SRS, YAML,
JSON, Clash-style text, dnsmasq text, or a text file that mixes such lines into an AdGuard list, whose bare names then
become `domain:` rules) is converted to AdGuard text: `domain:x` becomes
`||x^`, `full:x` a plain `x` line, `regexp:r` becomes `/r/`, `keyword:k` becomes `/k/` with the dots escaped (AdGuard
text has no keyword rule). Each converted line is read back with the AdGuard reader and kept only if it yields the same
rule (a rule with a control character is never written); the others are skipped. A block list takes only domain rules. An SRS block list is read strictly: a logical
rule is left out, and a rule with conditions besides domain items is left out and ends the reading (rules before it
are kept), both counted as skipped, because the block list must not match more than the file says (the routing sets
keep reading SRS as before). An AdGuard/hosts list is read at most up to 2,000,000 rules; more is a failed download.

**Line rules** (`text`, and each item of a YAML `payload`): blank lines, comments (`#` `!` `//` `;`, and a trailing
` #` comment) and `[...]` section headers are ignored.

| Line | Rule |
| --- | --- |
| `domain:x` `full:x` `keyword:x` `regexp:x` | as written (a trailing `@attribute` is dropped; names are lowercased; a keyword keeps its dots) |
| `DOMAIN` `HOST` | `full:` |
| `DOMAIN-SUFFIX` `HOST-SUFFIX` | `domain:` |
| `DOMAIN-KEYWORD` `HOST-KEYWORD` | `keyword:` |
| `DOMAIN-REGEX` | `regexp:` (skipped if the value contains a comma) |
| `IP-CIDR` `IP-CIDR6` `IP6-CIDR` | IP prefix (options after the value, such as `no-resolve` or a policy, are ignored) |
| other `TYPE,value` lines | skipped (`unsupported rule type`) |
| `+.x`, `.x` | `domain:x` (approximation: `.x` also matches `x` itself) |
| `*.x` and other `*` wildcards | an anchored `regexp:` in which a whole `*` label is exactly one label and `*` inside a label is any run within it |
| `server=/a/b/…` `address=/…` `ipset=/…` `nftset=/…` `local=/…` | `domain:` for each domain, the last segment is the target |
| `<ip> name [name…]` | `full:` for each name; `localhost` and the other local names are skipped |
| `\|\|x^` | `domain:x`; every other adblock form is skipped |
| address or CIDR | IP prefix (a bare address is /32 or /128) |
| bare `name.example` | `domain:` in `text`; `full:` in YAML `payload` (mihomo semantics) |

Names are ASCII with at least one dot where the form needs one; other lines are skipped. In an AdGuard
subscription (text kept as it is) a bare name is `full:` as before.

**YAML.** Only the top-level `payload:` list of scalars is read (plain, single- or double-quoted); the rest of the
document is ignored and no YAML feature (anchors, aliases, tags, flow lists) is interpreted, so a download cannot
expand. A document without `payload:` or with a flow-style list is refused.

**JSON** (sing-box source rule set, `{"version": n, "rules": [...]}`): headless rules with `domain` (`full:`),
`domain_suffix` (`domain:`; a leading dot is an approximation), `domain_keyword`, `domain_regex` and `ip_cidr`
(string or list). A rule with any other condition (port, process, `network`, …), `invert: true` or `type: logical`
is skipped whole, because it is narrower than the conditions kept; so is a rule that has both a domain condition and
`ip_cidr` (all conditions of one rule must match, which two separate sets cannot express).

**Switches and limits.** `regexp:` rules (also those made from `*` wildcards) are used only when the source has
`enable_regexp` (`sd_set_light`); a regexp with a control character (a line break would start another rule in the
converted file) or leading or trailing whitespace is skipped in every format; `adguard_rule` has no such switch and `/regex/` lines already apply there. Every
rule goes through the existing refusal check (valid-Go regexps outside the gate, Unicode-lowercase rules refuse the
whole download). Limits: the 64 MiB download limit, lines of 64 KiB or more are skipped, at most 2,000,000 rules per
file (more is a failed download, not a truncation). A download that cannot be read or yields no usable rule fails and
leaves the stored file and the rule count unchanged. Each successful download logs its format, the number of usable
rules and the number of skipped ones by reason.

**hosts lines in block lists (behavior change).** An `adguard_rule` line `0.0.0.0 name`, `127.0.0.1 name`,
`:: name` or `::1 name` (several names and a trailing comment allowed) now blocks each name as `full:name`;
`localhost` and the other local names are skipped. The reference and earlier versions of this host ignored such
lines. Hosts lines with any other address are still ignored.

### `.dat` (v2ray `geosite.dat` / `geoip.dat`)

One category is imported as a whole; the other categories are skipped by length without being decoded. The category
name is matched without regard to case; a file with the category more than once is merged. A missing category fails
the download and the message lists the first 20 category names of the file. `@attribute` filtering is not supported
(attributes are ignored). A domain set and a block list read a `geosite.dat`, an IP set reads a `geoip.dat`.

| `geosite.dat` Domain type | Rule |
| --- | --- |
| Plain (0, also when absent) | `keyword:` |
| Regex (1) | `regexp:` (needs `enable_regexp`; goes through the refusal check) |
| RootDomain (2) | `domain:` |
| Full (3) | `full:` |
| any other value | skipped (`unknown domain type`) |

`geoip.dat`: every CIDR of the category becomes a prefix (host bits are cleared). A category with `reverse_match`
fails the download, because "everything but these networks" cannot be expressed. An entry whose address is not 4 or
16 bytes or whose prefix is longer than the address is skipped.

The protobuf is read flat over the downloaded bytes (no recursion): varints are at most 10 bytes, every length must
fit in the remaining data, wire types other than 0, 1, 2 and 5 and fields of the wrong wire type are errors. A
damaged file fails the download and the stored file is kept.

### `.mrs` (mihomo binary rule set)

A zstd stream (pure Rust decoder), decoded through a hard limit of 64 MiB: more output fails the download, and a
frame that declares a window larger than that is refused before anything is allocated. After the header (`MRS\x01`,
behavior byte, count, extra) the `domain` behavior holds a succinct domain set and the `ipcidr` behavior address
ranges. The behavior has to match the subscription (a domain set or block list takes `domain`, an IP set `ipcidr`),
otherwise the download fails; `classical` is not an `.mrs` behavior. The succinct set is walked with the same bounded
traversal as SRS (bitmap and label sizes, key length, 2,000,000 keys, 64 MiB of keys).

mihomo stores each domain pattern reversed, and `DomainSet.Has` gives the patterns these meanings:

| Pattern in the file | Matches | Rule |
| --- | --- | --- |
| `x` | `x` only | `full:x` |
| `+.x` (stored as `x` and `+.x`) | `x` and its subdomains | `domain:x` |
| `.x` (stored as `+.x` only) | subdomains of `x` only | `regexp:^.+\.x$` (needs `enable_regexp`; `domain:x` would also match `x`) |
| `*` as a whole label (`*.x`, `a.*.x`) | exactly one label there | an anchored `regexp:` with `[^.]+` (needs `enable_regexp`) |

Approximation: mihomo's `*` can match an empty label in corner cases; here it needs at least one character.
`ipcidr` ranges are converted to the fewest CIDR prefixes (an IPv4-mapped IPv6 address is read as IPv4; a range
with mixed families or `from` after `to` is skipped as `invalid range`).

The same rule limits as the other formats apply (2,000,000 rules per file, a download that yields no usable rule
fails). Real-file comparison against the text versions published with `meta-rules-dat` is in the task validation
record (`geosite.dat`/`geosite/*.mrs` give the same domain rules, `geoip.dat`/`geoip/*.mrs` the same address ranges).

Not supported: `.mmdb` and `db` are not planned.

## Management (M1-a2)

The reference API is served under `/plugins/{tag}` for each provider tag; methods, paths, status
codes, content types and JSON bodies follow the reference handlers (task research
`go-semantics.md`):

- `sd_set_light`/`si_set`: `GET /config`, `PUT|DELETE /config/{name}`, `POST /update/{name}` (202,
  background). `PUT` with non-blank `files` and `url` downloads, then reloads. A source (and an AdGuard rule) has an optional
  `format` field, see "Subscription formats".
- `adguard_rule`: `GET|POST /rules`, `PUT|DELETE /rules/{id}`, `POST /update`, `POST /update/{id}`.
  The WebUI addresses the fixed tag `adguard`.
- `domain_set_light`: `GET /show` (`q`, `limit` default 100, `offset`), `GET /save`, `POST /post`
  (`{"values": [...]}`), only when the first file is a `.txt` file.

Manifests are kept in memory, written as the reference writes them (indented, sorted by name, or
by id for AdGuard; zero AdGuard `last_updated` omitted) through a same-directory temporary file and
a rename, and only then adopted in memory; a failed write changes nothing (500). Manifests written
by either implementation are read by the other (round-trip evidence in the record). Downloads:
HTTP/HTTPS with system roots, at most 10 redirects, optional SOCKS5 (no authentication, remote
name resolution), 60 s (`sd_set_light`/`si_set`) or 30 s (AdGuard) overall, 64 MiB body limit;
validated (SRS decode, or conversion of the other formats) before the rule file is replaced by rename. Downloads of one source never
overlap: a request during a download makes it run once more. Auto-update uses the reference
cadence (first check 1 min after start; sources every 10 min, AdGuard every minute) and predicate.
Errors are logged, as in the reference.

Hot reload: after a change the provider's own state (rule counts, the `si_set` list, the AdGuard
direct matcher) and every `domain_mapper` naming it are rebuilt off the DNS thread (one dedicated
build thread, so builds never run concurrently) and published as a whole generation; a query keeps
the generation it read; a failed build keeps the previous generation. Caches are not flushed (the
reference does not either). Shutdown cancels downloads and joins every job inside the host drain.

Deliberate differences (data safety or not configuration):

- A manifest that exists but cannot be parsed is never overwritten; changes to that provider fail
  (the reference starts empty and overwrites it on the next change).
- `domain_set_light` `/post` values are normalized like file lines (trim, skip blank and `#`, drop
  values with a line break), as approved for `domain_set` (A1); `/save` refuses a non-`.txt` first
  file (the reference would overwrite an SRS file with text); after `/post` the provider holds what
  a restart reads (the reference drops `exps` from memory until restart).
- Downloaded content and posted values that this host would refuse at start (valid-Go regexps
  outside the gate, Unicode-lowercase rules) are rejected, so a change can never make the next start
  fail. AdGuard text downloads are otherwise stored as they are, as in the reference; a download that
  yields no usable rule at all is refused (the reference stores it).
- `DELETE` keeps a rule file that another source of a configured provider still reads, however its
  path is spelled or linked (the reference deletes it and the other source loses its rules).
- A download whose source was removed or repointed meanwhile is discarded; files are always
  replaced by rename (the reference writes SRS files in place).
- No `HTTP(S)_PROXY` environment proxy (the reference's default transport honours it); use `socks5`.
- New `last_updated` values are written in UTC (`…Z`); the instant is the same.
- Auto-update reloads only when a download succeeded (the reference reloads after every batch).
- Start-up count reconciliation rewrites only counts read while compiling (enabled sources); the
  reference also recounts disabled AdGuard lists at start.

Known limits of a generation apply (the removed `native_management` mode shared them): an apply compiles a new
view with its own rule-source state. When the new view is installed the retired view is fenced at
once (no new jobs, no writes, mutating requests 503) and its jobs are joined during retirement;
every view's owner of one manifest or rule file shares that file's write lock, so a write already
under way finishes before the new view's first write and is never applied after it. A background rule write that
lands between a managed candidate's compile and its commit makes that apply fail with the existing
retryable input conflict. `domain_set_light` with `exps` or several files: `/show` lists `exps` and
every file and `/post` writes the first file, so posting the shown list back duplicates them there
(the reference has the same result after a restart).

Memory: a rebuild of a large mapper briefly holds the old generation (queries may still use it), the
rule text, the build's temporaries and the new generation. After publication the host returns the freed
pages to the system (glibc `malloc_trim` on the build thread; a fixed glibc mmap threshold and no
transparent huge pages for the process keep them returnable), so with the production rule set the
resident size settles near the start-up level after any number of rule-source rebuilds (numbers in
`docs/rust/validation-records/10-08-mem-1/`). The replaced generation is freed off the DNS thread. (The former
`native_management` apply, which was not followed by a trim, was removed in Phase 6b.)
