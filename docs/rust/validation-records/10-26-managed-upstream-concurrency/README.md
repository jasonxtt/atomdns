# Managed upstream concurrency and DDNS placement (2026-10-26)

User-authorized patch after memory optimization production trial. Parent:
`99934eea`. The user selected manual review and will return findings; no external
PASS is claimed. No production update, main merge, push or release in this patch.

## Behavior

- A nonempty `POST /api/v1/upstream/config` keeps the group's settings list,
  including lists equal to YAML. Its presence records management intent.
- Settings-owned aliapi uses all enabled entries concurrently, capped at20,
  on both startup and hot apply. Disabled entries do not add concurrency.
- Empty-list saves restore YAML list and concurrency. YAML-only groups keep
  explicit `concurrent`; merely opening/installing the UI changes nothing.
- Generated special groups independently derive concurrency from their resolved,
  deduplicated enabled population. Empty inactive placeholders remain valid.
- The reference-settings importer preserves managed lists even when equal to
  YAML, and reports the derived concurrency instead of the old deviation.
- DDNS ListEditor moved from upstream settings to routing directly after
  rewrite. Its tag, backing list, save workflow and hint are unchanged; the
  priority flag stays in mode, with a link next to the editor.

Settings version, config schema, API request shapes and forward executor are
unchanged. Contracts: [settings](../../contracts/native-settings.md),
[special groups](../../contracts/native-special-groups.md),
[WebUI](../../contracts/native-webui-hosting.md).

## Validation

| Check | Result |
| --- | --- |
| RED: YAML2 plus managed3 | Failed with2 versus3 before implementation |
| RED: save list equal to YAML | Failed because settings dropped the list |
| RED: actual DNS fanout after UI save | Failed with only one of three peers receiving |
| RED: generated special group3 | Failed with fixed2 |
| RED: DDNS page order | Failed because routing lacked the editor |
| Local fresh native-host lib / typed settings / migration | 304+21+4 passed |
| Linux native-host unit and integration targets, serial | 720 passed,0 failed |
| Final Linux current-source lib / special / typed / migration | 312+10+21+4 =347 passed,0 failed |
| Frontend tests | 80 passed |
| WebUI production build | Passed |
| Workspace strict Clippy, formatting, diff whitespace | Passed |

The API/DNS regression saves a YAML-equivalent three-peer list, verifies all
three receive, disables one, verifies only two receive, restarts and verifies
that selection persists, then restores YAML and verifies only one attempt under
its original concurrency. Compile cases also cover YAML2, disabled entries and
21 enabled entries capped20. Existing rejected-change/write-failure tests pass.

## Retained limitations and attempts

- A local macOS special-group occupied-port assertion gives200 instead of409.
  The same test fails on unmodified parent99934eea at the same assertion;
  Linux's corresponding target passes all10 tests. No unrelated bind fix added.
- A baseline comparison reused local build artifacts and contaminated a later
  test link. Native-host artifacts were cleaned and the final local329 tests
  were freshly compiled. The contaminated result is not acceptance evidence.
- Initial Linux fixture transfer omitted root test fixtures and did not compile;
  they were restored. A broad run then encountered disk pressure; only owned
  artifacts were moved/removed, with existing services preserved. A retry using
  two build jobs under memory pressure was killed; final focused347 passed
  using one build job after clearing unneeded artifacts.
- The workspace serial attempt completed1004 tests, including all720 native-host
  tests, before its240s limit in unchanged upstream-core. A separate upstream-core
  attempt also timed out. Neither incomplete attempt is a full-workspace PASS.
  No test assertion failures were recorded in those Linux attempts.
- Original memory acceptance applies to frozen candidate b16e850d, not a new
  performance measurement of this changed fanout policy. Production still runs
  `v0.4.0-memory-99934eea` until separately authorized replacement.

Raw logs are retained outside Git in the private evidence directory for this
patch. The owned Linux source/target and RAM-artifact directories were removed
after checks. The four pre-existing dirty files were preserved unchanged.
