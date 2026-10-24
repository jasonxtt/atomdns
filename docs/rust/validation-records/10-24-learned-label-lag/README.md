# 2026-10-24：学习路由的审计标签即时更新（M2-4 #9）

基线 `45f8eb6928481da9a07e1c9d3225ad34e9ffc1d1`，独立工作树实施；没有修改 WebUI、提交、发布或部署。
用户确认了缓存契约的例外：当前学习标签排在缓存的其它标签之前，过期/flush 后消失；显式标签赋值仍覆盖 mapper 上下文。
契约：[route learner](../../contracts/native-route-learner.md)、[cache lifecycle](../../contracts/native-cache-lifecycle.md)。

## 原因与修复

`config/providers.rs` 的 mapper 每次都查询实时 learner，按类别顺序合并标签和来源；不是 matcher 缓存没有失效。
`execution.rs` 的普通缓存、分支缓存、直接 fallback 缓存命中随后用历史 `domain_set` 覆盖当前标签，却保留当前 `matched_rule_source`，造成两者不一致。

- mapper 携带本条查询已有的学习标签前缀长度及共享的类别/默认标签名，缓存恢复时合并当前学习标签；不再次查 learner，不改变 marks 或学习确认。
- 显式赋值和原有作用域清理同时清理上下文；无输出的后续 mapper 不清掉前一个 mapper 的上下文。
- 类别标签按现有规则去除边缘空白；复合默认标签按 `|` 拆分。默认标签仅在当前学习命中时被替换，没有学习时仍保留缓存原有标签。
- 用应答原有标签派生 `effective_tag`，并用于所有缓存发布及 lazy refresh；临时学习注记不会经嵌套缓存固化成新的应答路由来源。
- API、指标、学习文件和缓存文件格式没有新增字段。没有删除缓存、重新构建 matcher 或增加 learner 查找；本次没有进行吞吐/延迟基准。

| 改动文件 | 内容 |
|---|---|
| `rust/native-host/src/config.rs`、`rust/native-host/src/config/providers.rs` | 编译共享标签信息，携带本次 mapper 学习前缀 |
| `rust/native-host/src/execution.rs` | 三处缓存恢复、四处发布，以及有效路由标签派生 |
| `rust/sequence-core/src/state.rs`、`rust/sequence-core/src/lib.rs` | 标签合并和应答标签投影，导出内部协作类型 |
| `rust/native-host/tests/route_learner.rs` | 两项 DNS/审计回归，涵盖五种执行情况 |
| `rust/sequence-core/tests/slice1_state.rs`、`rust/sequence-core/tests/slice4_safety.rs` | 默认标签回归，以及既有状态字面量补齐 |
| 两份契约、本记录、`docs/rust/plans/m2-backlog.md` | 用户批准的规则、证据和 #9 状态 |

## 最小复现与回归

真实 loopback UDP 查询和管理 API，只有时钟使用已有测试接口：

1. 首次查询写入静态标签的应答缓存并学习；同一代第二条查询命中缓存，来源已有学习类别，标签也必须带学习前缀。RED：`STATIC`，预期 `DIRECT|STATIC`；修复后 GREEN。
2. 普通缓存、fallback 分支缓存、直接 fallback 缓存、嵌套缓存、没有默认标签的 mapper，分别覆盖学习与静态标签合并、类别顺序、学习后再填充缓存、过期和分类 flush。
   过期/flush 后不再出现失效学习标签，DNS 缓存仍命中；每条热查询只增加一次 learner hit。
3. 外层 miss / 内层 hit / 再次外层 hit 保持原应答的 `effective_tag`；RED：`DIRECT|STATIC`，预期 `STATIC`；修复后 GREEN。
4. 没有当前学习前缀时，即使 provider 当前静态标签改变，也保留缓存原有默认标签。该公开状态接口回归先 RED，后 GREEN。

新增 **3 项测试**：`native-host/tests/route_learner.rs` 两项（目标 7 → 9），`sequence-core/tests/slice1_state.rs` 一项（6 → 7）。
其它既有测试仅补齐新增内部字段的空值。

## 构建与检查

Linux VM，专属源码和 target 目录，离线、`-j2`、`CARGO_INCREMENTAL=0`。二进制都用 `scripts/build-rust-native.sh`，先构建 Vue，再构建 Rust。
VM 缺少的两个公开 npm 依赖由本地补齐后传入，VM 的安装/构建使用离线模式。

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --offline --locked -j2 --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --offline --locked -j2 --workspace --no-fail-fast` | **1501 passed，0 failed，0 ignored**（97 个目标） |
| `route_learner` / `slice1_state` 定向测试 | 9 / 7 通过 |
| `tools/feature-flags` / `tools/route-learner` Python 单测 | 5 / 7 通过 |
| 标准脚本构建基线与最终二进制 | 通过 |

本地与 VM 的 8 个改动 Rust 文件逐文件 SHA-256 相同；对排序后的路径/摘要 JSON 再取摘要：
`6592f26b79c24c3e733343ea5f0d1de4821a8642b054c9920db06ebb00d79713`。
二进制 SHA-256：

- 基线：`574e3f769ebea57c8e0061ccdcb1ef6c478a0a8db667464b6487d9af2f93ea16`
- 最终：`f4eb34a67d9c52d9c218d6dbf091df7675f64db511c08f54b50a00a54d8757cc`

## 等价 G1 差分

原 M2-1 的私有运行目录已不存在，因此没有复刻历史的 432 行数据集。本次使用 VM 已有的仓库外旧快照，以及相同 schema-3 配置和相同 G1 开关组合，隔离二进制修复本身的影响。
配置副本中三个较新的 UI 专用 `requery_on_change` 声明字段不被基线接受，准备时只删除这些字段；不改变规则、顺序、开关组合或原始快照。

复用 `tools/feature-flags` 的语料扩展和 `tools/route-learner` 的 rig、agent、比较器：500 个排名域名及既有受控探针、两个 loopback 客户端、所有监听器，另加 150 条 SOA/PTR/HTTPS 查询。
每侧三个 pass、每 pass **1,228 行**，共 **3,684 行**。前两轮分别为正常监听器冷/热查询，第三轮为 requery；requery 未开启审计，只比较 DNS。
原始域名、地址、上游和查询行均留在仓库外临时目录，本记录仅包含数字与方法。

逐行比较 rcode、答案（忽略 TTL）、`domain_set`、`effective_tag`、`final_sequence`、最终上游组和规则来源；另比较最终学习集合。
学习前缀 oracle 从类别配置及 `matched_rule_source` 推导期望顺序；配置里显式 `tag_setter` 的标签不参加此前缀断言，但这些行仍完整参加所有差分比较。
每个正常 pass 有 24 行显式覆盖；两轮共有 **2,432 行审计**，规则来源字符串全部精确相同。

| pass | 行数 | 基线真实标签滞后 | 最终标签滞后 | 仅 `domain_set` 改变 | 其它比较字段差异 |
|---|---:|---:|---:|---:|---:|
| 正常冷查询 | 1,228 | 0 | 0 | 0 | 0 |
| 同代正常热查询 | 1,228 | **471** | **0** | **471** | **0** |
| requery（无审计） | 1,228 | — | — | — | 0（DNS） |

最终学习集合差异 **0**。修正后的差分与最终源码复跑得到相同结果。

对照：

- 基线 / 基线：两侧都检测出 471 行真实滞后，彼此字段差异 0；oracle 按预期返回失败，证明能识别原问题。
- 最终 / 最终：标签滞后 0、字段差异 0、学习集合差异 0，返回通过。

### 保留的失败与判据纠正

首次 oracle 报告 509 行（16 + 493），其中 **38 行**是显式标签覆盖（16 + 22），按批准的契约应当保留，并非滞后。
纠正是按配置识别显式覆盖，没有放宽任何数值阈值，也没有把这些行从逐行差分中移除。
初版候选消除了 471 行真正滞后，但也改变了这些行的 `effective_tag`，因此自我 review 第一轮 FAIL；修复应答标签派生和嵌套发布后，其它字段恢复零差异。
修复中被后续源码取代的一次全量运行及一次构建主动停止，不作为最终通过证据。

## Review 与清理

用户指定当前会话自我 review，未调用 CLI 或独立审查代理。第一轮 **FAIL** 的有效路由标签、嵌套缓存和 oracle 问题已修正；第二轮核对这些修复、最终源码全量测试、差分与清理证据，结论 **PASS**。无待处理 P0/P1 或延期 P2。
所有差分 rig 已确认进程退出、命名空间删除、原服务标识不变；最终扫描没有本次残留进程。VM 任务目录和本地依赖暂存目录已删除，原始快照保留，清理前后内容摘要一致。
独立工作树及未提交改动保留，等待用户后续处理。
