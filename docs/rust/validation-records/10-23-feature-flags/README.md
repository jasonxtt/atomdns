# 10-23 M2-1：具名开关（`feature_flag`）取代 `switch1`–`switch17`

任务 `10-23-rust-native-feature-flags`，基线 `b70eeea7`（v0.2.2）；file 仓库 `ccc64c9`（只改工作树，未提交）。设计与用户决定见任务的 `research/design.md`（本地，不进仓库）。
构建、测试、演练、差分全部在 VM `mosdns-rust`（Linux，2 vCPU，离线，`-j2`，`CARGO_INCREMENTAL=0`）上进行；测量输入是仓库外的生产快照，本记录只有数字与方法，
不含快照中的域名、地址、上游或开关取值（快照里的 `rule/switchN.txt` 在每次运行前被替换为测试设定的值）。
契约：[native-feature-flags](../../contracts/native-feature-flags.md)（新）、[native-settings](../../contracts/native-settings.md)、[native-update](../../contracts/native-update.md)；
[native-switch-state](../../contracts/native-switch-state.md) 改为"已删除"说明。

## 改动

| 位置 | 内容 |
| --- | --- |
| `rust/native-host/src/config/flags.rs`（新） | `feature_flag` 声明与校验、`flag key[=option]` 的常量答案、`settings.json` `flags` 段的校验与叠加、一次性迁移表与旧文件读取、启动迁移写盘（试运行期间不写） |
| `config.rs`、`config/settings.rs`、`config/check.rs` | 插件类型 `feature_flag`；`switchN` 类型与匹配写法改为加载错误；`Settings.flags`（`Option`，空段保留）；`EFFECTIVE feature_flag` 报告项；路径碰撞函数从 `switch.rs` 移入 `config.rs`（内容不变） |
| `config/standard_groups.rs` | 生成的 `sequence_special_N` 写 `matches: flag lazy_cache` |
| `settings_api.rs`、`api.rs` | `GET /api/v1/flags`、`POST`/`DELETE /api/v1/flags/{key}`；`flags.manage`；删除 `/plugins/{switch}/show|post`、`X-Mosdns-Config-Generation` 解析、`switches.manage` 与 `switches` 能力扩展 |
| `assembly.rs` | 启动编译后调用迁移写盘 |
| 删除 | `switch.rs`、`switch_state.rs`、`switch_admission_tests.rs`、`tests/switch_declarations.rs`；`runtime_snapshot.rs`/`udp.rs`/`tcp.rs`/`execution.rs` 的注册表与准入种子；sequence-core 的 `AdmissionFacts`（含 `tests/admission_facts.rs`） |
| `update/mod.rs`、`scripts/rust-release-metadata.json` | 配置 schema 3、`rust-config-schema-3` |
| 测试 | 新 `tests/feature_flags.rs`（14 项）、`tests/feature_flags_trial.rs`（1 项，独立进程：环境变量）；`m1_e_standard_groups.rs` 新增 1 项并改用 `lazy_cache`；`config_check.rs`、`m1_0_config_loading.rs`、`slice6_management_http.rs`、两个 fixture、`f3a_update.rs`（schema 常量）随之改写 |
| 工具 | `tools/feature-flags/`（新，差分）；`tools/route-learner/agent.py` 的审计类型表加 SOA/PTR/HTTPS（原 A/AAAA 行为不变）。`tools/e2e-1/special_groups.py` 未改：它把同一份配置同时给 Go 与原生程序，无法使用 `feature_flag`，属于历史工具 |
| file 仓库（工作树） | `config_rust/sub_config/flags.yaml`（新，13 个开关）；删除 `sub_config/switch.yaml`；`config_custom.yaml`（include + 38 处）、`forward_1.yaml`（19）、`forward_nocn.yaml`（1）、`process_v4.yaml`（12）、`process_v6.yaml`（12）共 82 处引用；`README-rust.md` 一节；`build_config_rust_up.sh` 4 行（用户批准） |

### 引用对照（82 处，脚本逐行生成后核对）

| 旧 | 新 | 处数 |
|---|---|---|
| `switch1 'A'`（`fast_mark 2`/`3` 两条，只在 `sequence_6666`） | `flag block_empty` | 2 |
| `switch1 'A'`（`fast_mark 1`，`sequence_6666` 与 `sequence_requery` 各 1） | `flag block_blocklist` | 2 |
| `switch2 'A'` + `switch12 'B'`（同一条规则） | `flag client_routing=only_listed_proxy` | 2 条（4 处） |
| `switch2 'B'` + `switch12 'A'` | `flag client_routing=listed_direct` | 2 条（4 处） |
| `switch3 'A'` / `'B'` | `flag core_mode=compat` / `=safe` | 10 / 18 |
| `switch4 'A'` | `flag lazy_cache` | 16（另宿主生成每组 1） |
| `switch13 'A'` | `flag full_cache` | 4 |
| `switch5`/`6`/`7`/`8`/`16 'A'` | `flag block_qtypes`/`block_aaaa`/`block_ads`/`prefer_v4`/`ddns_foreign` | 各 2 |
| `switch9 'A'` / `'B'` | `"!flag cn_fakeip"` / `flag cn_fakeip` | 4 / 4 |
| `switch17 'A'` / `'B'` | `flag dns_mode=fakeip` / `=realip` | 2 / 2 |

规则条数、顺序与所在流程不变（switch2/12 合并的两条规则各少一个条件元素），审计里的规则身份 `inline:<tag>#<序号>` 不变。`flags.yaml` 的默认值等于原包 `rule/switchN.txt`。

## 构建与测试

| 命令（VM，`rust/`） | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --locked --workspace --no-fail-fast` | **1516 passed，0 failed，0 ignored**（98 个目标）；基线 `b70eeea7` 同一 VM：1559 passed，0 failed，1 ignored（98 个目标） |
| `python3 -m unittest discover -s tools/feature-flags`、`-s tools/route-learner` | 通过 |
| `rtdns check`（新 `config_rust` + 快照数据） | `gaps=0 cascades=0 globals=0 definitions=118`；旧 `config_rust` 在新程序里 `gaps=99`，全部指向"removed in configuration schema 3" |

测试数量：删除 59 项（`switch_state.rs`/`switch_admission_tests.rs` 单元 36、`tests/switch_declarations.rs` 18、sequence-core `tests/admission_facts.rs` 4、`slice6_management_http` 的开关 HTTP 1；基线唯一的 ignored 是开关崩溃子进程辅助项），
新增 16 项（`feature_flags` 14：声明错误 13 种、引用错误与旧写法、settings 错误、迁移表、switch2/12 九种组合、缺失/非法/超长、表不适配、`check` 只读、启动迁移一次与不重复、空段、HTTP 全流程与在途查询、并发、**R1**、迁移前的首次修改；
`feature_flags_trial` 1：试运行不写、503、最终启动写入；`m1_e_standard_groups` 1：未声明 `lazy_cache` 时专属组配置被拒）。1559 − 59 + 16 = 1516。

**R1 证据**（`feature_flags.rs` `other_settings_saves_keep_the_flags_section`）：`flags` 分别为 `{}` 与 `{"core_mode":"safe"}`，旧 `rule/switch3.txt` 故意与之相反；经 `POST /api/v1/overrides` 保存一次后文件仍含原样的 `flags`；重启后不重新迁移，`core_mode` 仍为 `compat` / `safe`。
另外 `the_flag_routes_change_the_next_query_and_store_only_differences` 把全部开关恢复默认后文件为 `{"version":1,"flags":{}}`；`startup_with_default_values_records_an_empty_flags_section` 证明迁移写盘保留已有的代理设置。
所有保存路径（代理/ECS、上游、域名学习开关、开关接口）都从当前代的 `settings` 克隆后修改；专属组保存不写 `settings.json`。

## 迁移与回滚演练（VM，真实二进制，仅 loopback 的网络命名空间）

步骤模拟更新守护的做法（守护本身由 `f3a_update` 测试覆盖）：v0.2.2 + schema 2 包（`webinfo/settings.json` 预置一个 RFC 地址代理覆盖），经旧接口改 switch3=B、2=B、12=A、16=A、6=A（测试设定）→ 备份包触及的文件 → 用 `build_config_rust_up.sh` 副本打出的 schema 3 包覆盖（18 个托管文件，`create_if_missing={}`，`delete_files=["sub_config/switch.yaml"]`）。

| 步骤 | 结果 |
|---|---|
| 候选试运行（带 `MOSDNS_UPDATE_GUARD_TXN`） | `migrated=false`，`core_mode=safe`（来源 legacy）、`client_routing=listed_direct`、`block_aaaa=true`；`POST`/`DELETE /api/v1/flags/*` 均 503；`settings.json` sha256 与更新前相同 |
| 回滚（只恢复包触及的文件，与守护相同） | v0.2.2 正常就绪，`switch3=B`、`switch12=A`，`switch.yaml` 恢复，`settings.json` 字节不变 |
| 再次更新、最终启动（无标记） | `flags={"block_aaaa":true,"client_routing":"listed_direct","core_mode":"safe","ddns_foreign":true}`，代理覆盖保留；接口 `migrated=true`、来源 settings |
| 改旧文件（switch3→A、switch6→B）后重启 | `flags` 不变、`core_mode=safe`、`block_aaaa=true`（不重复迁移）；17 个旧文件保留 |

## 差分（`tools/feature-flags`，BEFORE：v0.2.2 + 旧 `config_rust`；AFTER：本改动 + 新 `config_rust`）

方法见 `tools/feature-flags/README.md`。语料 500 个生产排行名字 × A/AAAA，加 e2e-1 的受控探测（黑/白/灰名单、realip、DDNS、cnfakeip 过滤、改写、广告），两个客户端源地址（`127.0.0.2` 在 `client_ip.txt`），
各监听端口，以及 150 行 SOA/PTR/HTTPS；每遍 1,228 行。两边从同一份学习状态起步（realip 5,009、fakeip 2,094、nov6 3,097、nov4 587、sv4 8）。每个组合：53 端口两遍（冷/热缓存）+ 刷新端口 3066 一遍，逐行严格比较
`rcode`、应答、`domain_set`、`effective_tag`、`final_sequence`、`final_upstream_group`、`matched_rule_source`（集合）与结束时的学习集合，并检查 AFTER 写出的 `flags` 等于独立计算的期望。

| 组合 | AFTER 取值来源 | AFTER 的 `flags` | 不同行（p1 / p2 / 刷新，各 1,228 行） | 学习集合 | 结果 |
|---|---|---|---|---|---|
| G0 全部默认 | 迁移 | `{}` | 0 / 0 / 0 | 相同 | PASS |
| G1 安全模式 + 名单内不走代理 + DDNS 国外 | settings | core_mode、client_routing、ddns_foreign | 0 / 0 / 0 | 相同 | PASS |
| S3 core_mode | 迁移 | `core_mode=safe` | 0 / 0 / 0 | 相同 | PASS |
| S17 dns_mode | settings | `dns_mode=realip` | 0 / 0 / 0 | 相同 | PASS |
| S9 cn_fakeip（反向） | 迁移 | `cn_fakeip=true` | 0 / 0 / 0 | 相同 | PASS |
| S8 prefer_v4 | settings | `prefer_v4=true` | 0 / 0 / 0 | 相同 | PASS |
| S16 ddns_foreign | 迁移 | `ddns_foreign=true` | 0 / 0 / 0 | 相同 | PASS |
| S1 旧 switch1（两个都关） | settings | `block_empty=false`、`block_blocklist=false` | 0 / 0 / 0 | 相同 | PASS |
| S5 block_qtypes | 迁移 | `block_qtypes=false` | 0 / 0 / 0 | 相同 | PASS |
| S7 block_ads | settings | `block_ads=false` | 0 / 0 / 0 | 相同 | PASS |
| S6 block_aaaa | 迁移 | `block_aaaa=true` | 0 / 0 / 0 | 相同 | PASS |
| S4 lazy_cache | settings | `lazy_cache=false` | 0 / 0 / 0 | 相同 | PASS |
| S13 full_cache | 迁移 | `full_cache=false` | 0 / 0 / 0 | 相同 | PASS |
| C1 (A,B) | settings | `client_routing=only_listed_proxy` | 0 / 0 / 0 | 相同 | PASS |
| C2 (B,A) | 迁移 | `client_routing=listed_direct` | 0 / 0 / 0 | 相同 | PASS |
| C3 (A,A) | 迁移 | `{}`（none） | 0 / 0 / 0 | 相同 | PASS |
| H 运行中切换（G1 的取值） | 旧接口 / 新接口 | 同 G1 | 0 / 0 / 0 | 相同 | PASS |
| N1 对照：AFTER 故意 core_mode=safe | settings | `core_mode=safe` | 67 / 68 / 78（应答、路由字段） | realip/fakeip 各差 34 | PASS（检出差异） |
| N2 对照：AFTER 故意 listed_direct | settings | `client_routing=listed_direct` | 18 / 18 / 6 | 相同 | PASS（检出差异） |

每组 4–11 s；所有运行的 `cleanup.json`：自有进程已退出、命名空间已删除、VM 原有 `mosdns` 服务 PID 与状态前后一致。
**敏感性**：相对 G0，BEFORE 侧第 1 遍不同的行数为 S3 75、S17 111、S9 937、S8 523、S16 18、S1 17、S5 138、S7 4、S6 536、S4 2、S13 22、C1 1041、C2 22、G1 81（C3 为 0，与"不区分"一致）——每个开关在语料上都改变了结果，"0 行不同"有意义。

**H 组的方法说明**：新程序改开关是整代热应用，旧程序切开关不是。两边都存在的既有现象：同一代里刚学到的名字，`matched_rule_source` 已显示学习来源，而 `domain_set` 要到下一代才带学习标签
（G1 中两边各 432 行如此，完全一致）。第一次运行 H 时只有 AFTER 换了代，因此第 2 遍有 455 行只在这些标签上不同（应答、`final_sequence`、上游均相同）。
现在 H 在切换后两边都做一次相同的设置热应用（代理覆盖），再比较，结果 0 行不同。这一既有现象属于 `route_learner`/mapper，不是本任务引入的，记入"需要用户决定"。

失败判据（任一项）：上述任一字段不同、DNS 错误、缺审计行、学习集合不同、AFTER 的 `flags` 与期望不符、宿主退出、残留进程/命名空间；对照组未检出差异。

## 审查

Codex（`gpt-6.1-sol`/medium，只读）共 2 轮。

- 第 1 轮 `FINAL: PASS`：无 P0/P1；2 个 P2（代码类，均已修）：`"flags": null` 被当作"未迁移"（已改为加载错误，加测试）；试运行期间值相同的 `POST`/已是默认的 `DELETE` 返回 200（已改为 503，测试改为期望 503）。
- 第 2 轮（只核对上述两条）`FINAL: PASS`：两条均已修好，未引入新问题。修复后全量测试 1516 passed、fmt/clippy 通过。
  这两处修复不涉及 DNS 路径；上面的差分与演练用的是修复前构建的发布二进制。没有留到 `deferred-p2.md` 的项目。
