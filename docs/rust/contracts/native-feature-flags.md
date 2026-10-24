# Native feature flags (`feature_flag`, M2-1, configuration schema 3)

Named, typed flags replace the plugin types `switch1`–`switch17` (their state files, owner registry, per-query admission seeds,
fast-flag bits and `/plugins/switchN/show|post` routes are removed). Evidence: [10-23-feature-flags](../validation-records/10-23-feature-flags/README.md).
The superseded contract is [native-switch-state.md](native-switch-state.md).

## Declaration

```yaml
- tag: core_mode                 # the key; ^[a-z][a-z0-9_]{0,31}$, unique like every tag
  type: feature_flag
  args:
    label: 核心运行模式           # required, non-empty
    description: …               # required, non-empty
    section: mode                # mode | upstream | routing | blocking | rewrite | learning | cache | client | special_groups
    kind: choice                 # bool | choice
    options:                     # choice only: at least two; values ^[a-z][a-z0-9_]{0,31}$, unique; labels non-empty
      - {value: compat, label: 兼容模式}
      - {value: safe, label: 安全模式}
    default: compat              # choice: one option value; bool: true/false
    requery_on_change: true      # optional bool, default false: see "Follow-up requery"
```
Any other `args` key, a `bool` with `options`, a default of the wrong type or outside the options is a load error (start, `check`, hot apply) naming the YAML path.
A flag is not executable and not a `$tag` matcher (`exec: $core_mode` / `matches: $core_mode` are errors that name the `flag` syntax).
`type: switchN` and the quick matcher `switchN …` are load errors that say they were removed in configuration schema 3.

## References

`flag <key>` matches when a `bool` flag is on (`"!flag <key>"` when off); `flag <key>=<option>` matches when a `choice` flag has that option.
An undeclared key, `=` on a `bool`, no `=` on a `choice` or an unknown option is a load error. The matcher's answer is fixed when the generation compiles
(trace name `anonymous_match(flag: …)`).

The host-generated special-group YAML (`sub_config/special_groups.yaml`) caches through `matches: flag lazy_cache`; a configuration with standard special groups
must declare a `bool` flag `lazy_cache` (otherwise it does not load).

## Values

`webinfo/settings.json` (see [native-settings](native-settings.md)) has a top-level `flags` object: `bool` flags as JSON booleans, `choice` flags as option strings,
**only the values that differ from the YAML `default`**. Unlike the other sections it is written even when empty (`"flags": {}`): its presence means the
schema-2 switch files were migrated. Every settings save (overrides, upstreams, domain generation, special groups, the flag routes, the migration) keeps it as it is.
A key that names no `feature_flag`, a value of the wrong type or option, or `"flags": null` (present but not an object) is a load error. Effective value: `flags[key]`, else (while `flags` is absent) the migrated
legacy value, else the default; reported as `EFFECTIVE feature_flag <key> value=<v> source=yaml|settings|legacy` by `check` and in `GET /api/v1/settings/effective`.

### Consistency
The values are constants of the compiled generation. A query keeps the generation it was admitted to through branches, fallback, preference, redirect and lazy
refresh, so all flags it sees come from one committed state and never change during it. A query that the host sends to one of its own listeners (requery, a
loopback upstream) is a new admission and sees the generation current then — as with the former per-admission switch seed. The host reserves no fast-flag bits;
`fast_mark`/mapper `mark` 32–47 and 49 are free for configurations (48 stays the `unified_matcher1` sentinel convention).

## HTTP

| Route | Behaviour |
|---|---|
| `GET /api/v1/flags` | `{"settings_file":"webinfo/settings.json","migrated":bool,"flags":[…]}` in declaration order; each `{key,label,description,section,kind,default,value,modified,source,requery_on_change}` plus `options:[{value,label}]` for `choice` |
| `POST /api/v1/flags/{key}` | Body exactly `{"value": <bool or option string>}`. Applies through the settings hot apply and answers the flag plus `requery_triggered` (200). The current value again: 200, nothing applied, `requery_triggered: false` |
| `DELETE /api/v1/flags/{key}` | Back to the default (the key leaves `flags`), 200 with the flag and `requery_triggered` |

Unknown key 404; bad JSON, extra fields, wrong type or option 400; other methods 405; a change while another is being applied or the file changed 409 (as the other
settings routes; a client should submit one change at a time); during an online-update trial 503 `配置更新试运行中，暂不能修改开关` (also for a request that would change nothing; malformed requests still get 400/404); write failure 500.
Before the first migration (only possible during an update trial or after a failed migration write) the first change stores all migrated values with it.
The routes exist when a file-backed configuration declares a flag; capability `ui_operations["flags.manage"]`. `switches.manage` and the `switches` capability
extension are removed. A change is a whole-generation hot apply: one compile (off the DNS thread), a short publication pause, and — accepted — a running `requery`
task is cancelled by the retirement of the old generation, as for any settings save. No cache is flushed; follow-up actions (flush, requery) are a client's choice.

## Follow-up requery (`requery_on_change`, 2026-10-24)

A flag may declare `requery_on_change: true` (a bool; anything else is a load error naming `args.requery_on_change`). When a `POST` or `DELETE` **changes the value**
and the new generation is published, the host then starts one task of the configuration's `requery` plugin on that new generation (the first declared one; the task flushes
the caches named by its own `flush_rules` and warms them up again, see [native-requery](native-requery.md)). The answer's `requery_triggered` says whether a task was started.
It is `false` when the value did not change (nothing is applied), the flag does not declare the parameter, the configuration has no `requery` plugin, or the task could not start
(the change stays applied; a warning is logged). A request refused with 400/404/409/503 starts nothing — in particular during an online-update trial (503) no task runs.
The WebUI therefore does no follow-up of its own (the former client-side "flush these caches and trigger requery" after a core-mode or DNS-mode change is gone). `config_rust`
sets the parameter on `core_mode`, `dns_mode` and `cn_fakeip`.

## Migration from `rule/switchN.txt` (one time)

At a normal start, after the compile and before serving, when `settings.json` has no `flags`: the values come from `<root>/rule/switchN.txt` (read up to 1 MiB,
surrounding blanks trimmed, exactly `A` or `B`) for the declared flags that fit this table, and `flags` (differences only, possibly `{}`) is written durably
(temporary file, fsync, rename) unless the file changed since it was read. The switch files stay and are not read again.

| key | from | mapping |
|---|---|---|
| `core_mode` | switch3 | A `compat`, B `safe` |
| `dns_mode` | switch17 | A `fakeip`, B `realip` |
| `cn_fakeip` | switch9 | **B on**, A off |
| `prefer_v4`, `ddns_foreign`, `block_qtypes`, `block_ads`, `block_aaaa`, `lazy_cache`, `full_cache` | switch8, 16, 5, 7, 6, 4, 13 | A on, B off |
| `block_empty`, `block_blocklist` | both switch1 | A on, B off |
| `client_routing` | switch2 + switch12 | (A,B) `only_listed_proxy`, (B,A) `listed_direct`, anything else `none` |

A missing, unreadable, oversized or non-`A`/`B` file keeps the YAML default and logs a warning (`client_routing` follows its row: such a value was "no distinction"
before as well). Under schema 2 such a value made both rules of an A/B pair miss; that state no longer exists. A declaration whose kind/options do not fit its row is
left alone; switch10/11/14/15 are not read.

**Update trial**: while the update guard tries the new program out (`MOSDNS_UPDATE_GUARD_TXN` set), the migrated values are used in memory only, nothing is written,
and the flag routes answer 503; the guard's final start (without the variable) writes them. An automatic rollback therefore finds `settings.json` unchanged,
which matters because the schema-2 program refuses an unknown `flags` key. `check` never writes. A manual downgrade to v0.2.2 must first remove `flags` from
`settings.json` ([release.md](../release.md)); deleting `settings.json` or its `flags` key makes the next start migrate the (stale) switch files again.

## Configuration package (schema 3)

`REQUIRED_CONFIG_SCHEMA = 3`, `rust-config-schema-3`. `config_rust` declares 13 flags in `sub_config/flags.yaml` (`switch.yaml` is in `delete_files`;
`create_if_missing` is empty; `rule/switchN.txt` are user data, never created or deleted by the package). See [native-update](native-update.md).
