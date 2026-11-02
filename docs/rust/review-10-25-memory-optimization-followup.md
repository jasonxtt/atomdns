# 工具修复复审报告 — 10-25 follow-up（109ecd7f）

> 外部只读复审交付件（第二轮）。复审范围：`9d6a1f67..109ecd7f`（分支 `codex/memory-optimization-validation`，唯一提交 `test(memory): verify review findings and harden diagnostic evidence`，HEAD `109ecd7fcf00a49182b7371cda3c557625ff9121`，复核时无漂移）。报告由用户手工转交，未向其他会话自动发消息。

**实际执行的复核**（全部只读，经 SSH 在 Mac 工作副本上进行）：

- 逐行读 `run.py`/`evaluate.py`/`prepare.py`/`diagnostic_candidate.py`/`audit_synthetic.rs` 及三个新测试文件（`test_evaluate.py`/`test_static_oracle.py`/`test_diagnostic_candidate.py`）的 diff；确认 `git diff 9d6a1f67..109ecd7f -- rust/` 为空（产品 Rust、公开 API、YAML、容量、持久格式零改动）。
- 在 Mac 上重跑：Python 单测 **19/19 通过**、`py_compile` 通过、`rustfmt --check`（fixture）通过。
- 交叉核验 `review-synthetic.json` 全部 22 条：Mac 本地 `/tmp/rtdns-memory-20261025/review-synthetic/` 日志与 **test-host `/root/rtdns-memory-20261025/` 原件逐字节同哈希（22/22）**；日志内 `total_requested_bytes`、`SYNTHETIC_INPUT` checksum 与 JSON 记录一致；两份测试二进制哈希与记录一致且互不相同（`543775b4…` vs `e31cd83d…`，文件大小与构建时间不同）；fixture SHA-256 与已提交文件一致；base/audit 副本的 `observer.rs` 源哈希按预期不同、`retention.rs` 按预期相同（该文件未被候选提交改动，仅两侧施加同一诊断变换）。
- 未执行（如实声明）：Rust 重编译、22 次 cargo 合成重放、smoke 重跑——依据的是已验证内部一致性与跨机身份的记录证据；未触碰生产服务、未修改任务/源码/配置、未启动正式长矩阵。

## 逐点独立结论（不采信原报告或主会话的归因结论）

1. **预填充阻断**：✅ 成立。`run.py:168-180` 记录 planned/sent/healthy/statuses，非诊断运行不满足即在主测量前中止；`evaluate.py:20-27,58-60` 对每行 `prefill_dns` 独立复检；wrong/late/unsent/missing 四类均有单测覆盖。`'sent'` 口径与 `analyze.window` 一致（`5d-1/load.py:178,181`：send_error/unsent 行 `sent=None`）。零容量仅在 `prefill==0` 时允许空（`allow_empty`），且测量窗口仍要求 planned>0；smoke/diagnostic 均被 `excluded_from_acceptance` 排除，无法借预填充缺口误判通过。
2. **`--top` 标注**：✅ 成立。identity 记录 `top_source` 与原始文件 SHA-256；evaluate 对一切非 `default`（含缺失时的 `unknown`）标 supplementary 并追加 reason，旧身份缺字段判 incomplete。
3. **route_learner 守卫**：✅ 保守且比旧版更强。现按“配置能力”阻断（已配置即阻断，空快照也阻断），旧 manifest 缺字段同样阻断。独立核验检测面：`prepare.py` 的 `definitions()`（主配置＋一层 include 的 plugins）与产品 `config.rs:1016-1031,1816-1827` 的收集语义一致（include 文件仅允许 `plugins` 键、无嵌套 include），且工具侧多扫标准 include 只会更保守。唯一“旧版阻断而新版不阻断”的角落（无 learner 配置但残留状态文件）无害——无插件则该文件根本不被读取。**确认没有任何 `last_seen` 改动，K1 仍处于阻断状态。**
4. **合成回放**：✅ 输入、哈希、身份齐全。两份一次性副本施加同一 DefaultHasher 变换（`diagnostic_candidate.py:31-38`），manifest 记录变换与 fixture 哈希；逐对 input checksum、42 模板/应答群体数量、逐字段保留字节相等（总差 = 容器差 = 精确 4,194,304 B / 16,777,216 B，全部来自重复时间队列容器）；22 次全部通过。DefaultHasher 只存在于诊断副本，且正确声明不得用于产品 CPU/延迟/RSS 或生产收益验收。
5. **数字与门槛**：✅ 19.21%/11.76%/9.43% 从日志逐一重算，算术全部正确（4194304/21830755、/35668792、/44467361；400k 16777216/87025501=19.28%）。ttl/unique 两组低于 15%，记录明确未择优、未宣布门槛通过；0/400k 明确只有单对诊断；产品 Rust 未改，且记录明确声明未把旧 1527/0 冒充本次重跑。
6. **P2-1/P3-6/7**：✅ 含结果池的 50% 索引门槛原样保留；未做任何未经测量的性能扩展优化。

另：`review-reconciliation.md` 对上轮“重基准 last_seen”建议的否决理由（重置会把已过期条目复活、抹平相对年龄；即便保留统一偏移也须处理过期集合与在跑学习/刷新，不能自动证明静态 oracle 有效）**独立成立**，本复审认可该否决；本轮扩大阻断是更安全的选择。

## 发现（按严重度；无产品缺陷、无工具缺陷级问题）

- **P3-1【工具，极低】** `run.py:171`：`planned=args.prefill+10000` 与 duration 公式（`run.py:172`）重复硬编码同一 padding 常量。触发：将来只改其一。影响：fail-closed（sent≠planned 报错中止），但报错语义模糊。最小修复：提为单一命名常量。
- **P3-2【工具，低】** `identity.top_sha256` 仅记录、evaluate 不复核（原始文件不在时只能如此）。影响：仅人工误读为“已校验”的风险。最小修复：文档注明“provenance 记录项，非校验项”，可不改代码。
- **P3-3【证据观察，已核验非缺陷】** ttl-base 第 2/3 次日志逐字节相同（含 `finished in 1.76s`）。test-host 原件同样相同；其余 20 个日志哈希互异、两份二进制互异、构建时间相邻——判定为确定性输出＋时长巧合，**无日志复用或伪造证据**，无需行动，仅记录。
- **证据缺口（与记录一致，非新缺陷）**：生产结果池基数未测（P2-1 保留）、shared 19.21% 不能外推生产 15% 门槛（profile 依赖已实证）、0/400k 仅单对、K1 状态感知模型未设计、正式三组配对矩阵/热应用峰值归因/两小时稳定性未运行。

## 结论

**REVIEW PASS**（仅限本次工具变更 `9d6a1f67..109ecd7f` 的范围：预填充正确性门槛、语料 provenance、配置化 learner 阻断、合成诊断回放及其证据链。不覆盖任何产品 Rust 改动——本次没有；不覆盖任何未运行的正式验收门。）

**MERGE READY: NO**（独立结论：K1 未解决，全部正式验收门仍未运行，与记录结论一致，不能合并 main。）
