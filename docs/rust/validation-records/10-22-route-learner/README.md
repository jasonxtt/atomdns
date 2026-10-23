# 10-22 M2-2：声明式路由学习（`route_learner`）

任务 `10-22-rust-native-route-learner`，基线 `5e2412c6`（v0.2.1 之后的纯文档提交），设计与决定见任务的 `research/design.md`、`phase2-go.md`（本地，不进仓库）。
构建、测试、差分全部在 VM `mosdns-rust`（Linux，2 vCPU，离线，`-j2`，`CARGO_INCREMENTAL=0`）上进行；测量输入是仓库外的生产快照，本记录只有数字、方法和语料序号，不含快照中的域名、地址或上游。
契约：[native-route-learner](../../contracts/native-route-learner.md)（新）、[native-domain-output](../../contracts/native-domain-output.md)、[native-requery](../../contracts/native-requery.md)、[native-update](../../contracts/native-update.md)。

## 改动

| 位置 | 内容 |
| --- | --- |
| `rust/native-host/src/route_learner.rs`（新） | `route_learner`（存储：每类一张表，键为规范化小写无尾点的域名；条目有首次/最近确认、命中次数、原因；每条按类别 `ttl` 单独过期；容量上限与淘汰；一个 JSON 状态文件，有变化才写；内存交接；`show`/`save`/`flush`/`entries`/`status`）与 `route_learn`（规则：`when` 条件 + `learn` 类别） |
| `config.rs`、`config/providers.rs`、`config/settings.rs` | 两个新插件类型的编译与校验；`domain_mapper` 以学习层取代 `HotOverlay`/`HotLayer`（合并语义不变，另带 `source`）；设置页开关对类别生效；`domain_output` 拒绝 `target_mapper`/`target_mark`/`target_tag` |
| `execution.rs`（3 处调度）、`runtime_snapshot.rs` | `route_learn` 作为外部可执行体调度；热应用发布时 `adopt` 旧代学习状态（内存交接）后再 `fence` 旧代 |
| `api.rs`、`settings_api.rs` | 类别与学习器的路由、能力判断、域名生成开关的 flush 目标 |
| `requery.rs` | `flush_rules` 里指向学习类别的 `/plugins/<tag>/flush` 被跳过（缓存等其它 URL 照旧执行），新增加性状态字段 `skipped_flush_urls` |
| `domain_output.rs` | 删除 `MapperTarget`/热层注入与载入时注入（死代码），其余不变 |
| `update/mod.rs`、`scripts/rust-release-metadata.json` | 配置 schema 2、`rust-config-schema-2` |
| `webui-log/…/DataManagementManager.vue`、`rust/native-host/ui/` | “保存/清空全部”路径表去掉 `my_nodenov4list`、`my_nodenov6list`、`my_notinlist`（共 6 行），前端重建（`app.js` 少 135 字节，两个 `index.html` 只换缓存戳） |
| `tools/route-learner/`（新）、`tools/e2e-1/mock.py` | 差分工具；mock 增加 `empty` 故障模式（NOERROR、无记录） |
| `file` 仓库（只改工作树，未提交） | `config_rust`：新 `sub_config/route_learner.yaml`；`forward_1`/`process_v4`/`process_v6` 的 27 个学习位置各改为 `exec: $learn_xxx` 一行；`domain_output.yaml` 只剩 `top_domains`；`rule_set.yaml` 去掉 7 个 `my_*rule` provider 与 5 条 mapper 学习规则；`config_custom.yaml` 增加 include；删除 16 个 `gen/` 样例文件；`README-rust.md`；`build_config_rust_up.sh` 只改 `SCHEMA=2`、`PACKAGE_ID="rust-config-schema-2"` 两行（`config_rust_up.zip` 未动） |

## 构建与测试

| 命令（VM，`rust/`） | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --locked --workspace --no-fail-fast` | **1559 passed，0 failed，1 ignored**（98 个目标）；基线 `5e2412c6` 同一 VM：1547 passed，0 failed，1 ignored（97 个目标）；净增 12（新增 14：单元 7、集成 7；删除 2 项学习注入用例：`domain_output.rs` 单元 1、`m1_c1_domain_output` 集成 1；`f3a_update`/`m1_c1` 其余按 schema 与新语义改写） |
| `rtdns check`（新 `config_rust`） | `gaps=0 cascades=0 globals=0 definitions=122`；旧 `config_rust` 在新程序里：`gaps=5`（`domain_output` 的 `target_*` 被拒绝，错误指向 `route_learner`） |
| `python3 -m unittest discover -s tools/route-learner`、`tools/e2e-1/test_mock_faults.py` | 通过 |
| 配置包试构建（`build_config_rust_up.sh` 的副本，输出到仓库外临时目录） | `schema=2`、`package_id=rust-config-schema-2`、18 个托管文件（含 `sub_config/route_learner.yaml`）、`delete_files` 为空；原 `config_rust_up.zip` 未动 |

新增测试：`route_learner.rs` 单元 7 项（规范化与类别顺序、逐条过期、容量淘汰、关闭类别、读写回环、损坏文件/超容量/未知类别、交接共享状态）；集成测试 `tests/route_learner.rs` 7 项
（规则条件与下一次查询的路由、与 mapper 的合并语义与默认 mark、`show`/`save`/`flush`/`entries`/`status` 与方法/状态码、关机写盘与重启即时生效与损坏文件、热应用交接与设置页开关、配置错误 10 种加第二个学习器、requery 的 flush 规则）。
改动的既有测试：`m1_c1_domain_output`（删除依赖 `target_mapper` 的 1 项、改写 2 项，增加旧参数被拒绝）、`f3a_update`（schema 常量）。回归证明：把 `runtime_snapshot.rs` 里的 `adopt` 一行注释掉，热应用交接测试失败。

## 差分（BEFORE：`5e2412c6` + 旧 `config_rust`；AFTER：本改动 + 新 `config_rust`）

工具与判定见 `tools/route-learner/README.md`。语料 2,000 个生产排行名字 × A/AAAA 加受控探测与各监听端口，每遍 4,078 行；每个宿主在自己的 loopback 命名空间里与独立 mock 一起运行，串行。
比较字段：`rcode`、应答记录（不含 TTL）、`domain_set`、`effective_tag`、`final_sequence`、`final_upstream_group`，以及每遍后各类别的学习集合；`matched_rule_source` 见下。

| 组 | 结果 | 数据 |
| --- | --- | --- |
| A 冷启动（含 C 重启） | **PASS** | 第 1、2 遍各 4,078 行六个字段全部相同；每遍后的学习集合相同（realip 1,709、fakeip 304） |
| C 重启（在 A 里） | **PASS** | 优雅停机、清缓存 dump、重启：4,078 行无差异；两边都带回停机时的集合 |
| B 学习后（生产规模，BEFORE 旧文件 ↔ AFTER 转换的状态） | **PASS** | 起始集合 realip 5,009、fakeip 2,094、nov6 3,097、nov4 587、sv4 8，两边完全相同；两遍各 4,078 行无差异；三个时点的学习集合相同（结束时 realip 6,303、fakeip 2,332） |
| D 过期（AFTER 全部 `ttl` 60 s） | **PASS** | 学习遍与 BEFORE 无差异；等待 65 s、清缓存后 AFTER 的学习集合为空，第二遍对 BEFORE 冷启动第一遍 4,078 行无差异（过期 = 从未学过） |
| E SERVFAIL（`domestic` 组全部 SERVFAIL） | **PASS** | 学习集合的差异只有 BEFORE 把 **1,573** 个 SERVFAIL 名字学成 nov4/nov6 而 AFTER 没有（无“AFTER 多学”）；有差异的行全部属于这些名字且不含 DNS 错误/缺审计（第 1 遍 1 行、第 2 遍 3,148 行——第 2 遍前先 `save`+发布：这些名字在 BEFORE 里被记忆无 V4/V6 空答，AFTER 仍如实转发），其余 0 行 |
| F 热应用（负载下 6 次设置保存） | **PASS** | 4,078 行 0 个 DNS 错误、0 次保存失败、已学名字 0 丢失；应用期间延迟 p50/p99/max 0.3/0.7/4.1 ms（同负载无应用 0.1/0.2/1.5 ms；旧程序同样 6 次应用 0.3/0.9/3.7 ms；工具串行发送、约 1 个并发，与 `10-22-offthread-apply` 的 500 qps 开环负载不可直接比较，只作无停顿的证据）；RSS（负载后 30 s）53.1 MiB（54,404 KiB），同负载无热应用的对照 45.8 MiB（46,948 KiB），比值 1.16（≤1.2）；旧程序同样 6 次应用 53.2 MiB（54,448 KiB） |
| G 刷新流程与负向学习（`domestic`，再 `domestic`+`foreign`+`foreignecs` 返回空 NOERROR；整个语料送到 `sequence_requery` 监听） | **PASS** | 三遍各 4,078 行无差异；每遍后学习集合相同；学到 fakeip 2,018、nov4 1,573、nov6 1,573、realip 1,573、sv4 1,714（五个类别都被覆盖） |

`matched_rule_source`：
- **稳态（第二遍与 G 的普通流程遍，BEFORE 已把学到的名字发布进规则 provider——旧程序要等约 10 小时的周期写盘，工具用各类别的 `/save` 加把规则文件 POST 进 `my_*rule` 模拟这一状态）按严格口径比较**：所有行集合相同；
  只有顺序不同（AFTER 的学习来源固定在最前，旧程序按 matcher 规则顺序）：A 34、B 460、G 3,158 行；差异只出现在名字同时是更靠前静态名单里的 `full:` 条目时。
- **学习遍（A、B、D、E 的第 1 遍；BEFORE 刚学到的名字只在内存覆盖层里，没有来源，审计回退到序列规则的 `inline:` 身份，下一次发布前一直如此；AFTER 的学习层总是带来源）放宽**：删去五个学习来源名和 `inline:` 后相同；
  这类行 A 第 1 遍 2,029、B 第 1 遍 1,540、D 2,029。这是旧程序暂态与新程序的一处**有意的差别**（设计 D6），不是回归；其它来源的集合在所有行上相同。

失败判据（任一项）：六个字段任一不同、DNS 错误、缺审计行、学习集合不同（E 组仅允许上述 nov4/nov6）、`matched_rule_source` 其它来源的集合不同、宿主退出、残留进程/命名空间。

内存（稳态，学习名字发布后）：A 组 AFTER 46.3 MiB 对 BEFORE 55.0 MiB；B 组（生产规模约 1.1 万条）AFTER 46.9 MiB 对 BEFORE 61.0 MiB（BEFORE 的 matcher 里多出这些 `full:` 规则与统计表、热层的副本）。

### 清理
所有运行的 `cleanup.json`：自有进程已退出、命名空间已删除、原有 `mosdns.service` 的 PID 与状态前后一致。

## 审查

Codex（`gpt-6.1-sol`/medium，只读）共 3 轮。

- 第 1 轮 `FAIL`：1 个 P1（差分对 `matched_rule_source` 的允许差异过宽——学习来源与 `inline:` 身份在所有遍里都被忽略）、4 个 P2。
  P1 的处理：只在学习遍放宽（旧程序刚学到的名字只在内存覆盖层里、没有来源，审计回退到 `inline:` 身份，这是旧程序的暂态，不是新程序的回归）；第二遍前对两边调用各类别 `/save` 并把旧程序写出的规则文件 POST 进 `my_*rule`
  （复现周期写盘后的稳态），第二遍严格比较。P2：旧代在途查询的学习在热应用后被丢弃（已改：已交接状态的旧代继续写入共享状态，加测试）；
  每周 requery 续不了记忆无 V4 的期（已改：`config_rust` 的 `sequence_requery` 去掉 `fast_mark 2/3` 的提前空答，使刷新重新探测并再确认）；E 组把 DNS 错误/缺审计也算作允许差异（已改：硬失败不豁免，加测试）；
  F 组没比应用窗口延迟（已加：p50/p99/max，对照无应用与旧程序，max < 2000 ms 为判据）。
- 第 2 轮 `FAIL`：4 个 P2 已确认修好；P1 仍有遗漏——E 组第二遍前没有 save/发布、两遍都放宽。已改并让七组用最终二进制重跑（上面的数字）。
- 第 3 轮 `FINAL: PASS`，无新问题。没有留到 `deferred-p2.md` 的项目。
