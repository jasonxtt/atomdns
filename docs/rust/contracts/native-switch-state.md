# Native switch state (removed in configuration schema 3)

The plugin types `switch1`–`switch17`, their `rule/switchN.txt` state files, the host switch registry, the per-admission switch values and fast-flag bits
32–45/47/49, the `/plugins/{switch tag}/show|post` routes with `X-Atomdns-Config-Generation`, the `switches.manage` operation and the `switches` capability
extension were removed by M2-1 (2026-10-23). Named flags replace them: [native-feature-flags.md](native-feature-flags.md), which also describes the one-time
migration of the `rule/switchN.txt` values.

This file stays so that older validation records keep a target. The removed behaviour is recorded in git history (up to `b70eeea7`) and in
[S4](../validation-records/10-05-rust-native-switch-state-management/s4-status.md),
[S5](../validation-records/10-05-rust-native-switch-state-management/s5-status.md) and
[S6](../validation-records/10-05-rust-native-switch-state-management/s6-status.md).
