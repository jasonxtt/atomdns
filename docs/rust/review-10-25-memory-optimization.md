# 首轮代码／测量工具审查报告 — 10-25 内存优化候选

> 外部只读审查交付件。审查会话未修改代码、任务、分支或配置；报告由用户亲自转交主会话。

**审查范围**：`90997571..cbf66577`（BASE→HEAD，5 个提交），只读。本机复核情况：工作区 `rust/`、`tools/` 与 HEAD 逐字节一致（`git diff cbf66577 -- rust/ tools/` 为空）；测量判定器单测重跑 11/11 通过；**本审查环境无 Rust 工具链（无 cargo）**，Rust 测试／Clippy 结论依据 `verification.json` 记录的日志哈希与源码逐行通读，未能在本机重跑——如实声明，不作猜测。未访问生产、未部署、未清理用户脏文件（`m2-backlog.md`、`PLANNING-PACKAGE-CHANGES.md` 未动）。

---

## 一、发现（按严重度排序）

### P2-1【测量工具／验收证据代表性】索引堆 ≥50% 门槛对“结果池基数”不敏感

- 位置：`tools/memory-validation/index-probe/main.rs:84`（`ValuedRule::new(rule, 1<<i, [i as u32], source, source)`，payload 只按输入文件区分，226,496 条规则仅产生 24 个 pooled results）；冻结门槛 `docs/rust/validation-records/10-25-memory-optimization/README.md:11`（"Index live requested heap including result pool: at least 50% reduction"）。
- 触发条件：用该探针结果作为 ≥50% 索引堆门槛的证据。
- 实际影响：BASE 与候选都含同一个（极小的）结果池，30.24→8.11 MiB 的差几乎全部来自索引结构本身。生产 mapper 的共享 payload 基数（distinct fast_marks/ctx_marks/tags/sources 组合数）未被测量；若生产基数达数万，池在两侧同额增长，含池比例的 50% 门槛结论可能与纯索引结论相反（或意外通过）。记录里已声明“不是完整生产 mapper census”，但**冻结门槛文本没有固定池基数**，该缺口未闭环。
- 最小修复：从生产快照统计（免名字的）payload 组合基数，探针按该基数生成代表性 payload；或把门槛拆成“纯索引字节下降 ≥50%（探针可证）＋ 生产池基数实测记录”两条，写入验收定义后再采信。

### P2-2【测量工具】审计 100k 堆对照不可归因的根因是“经真实 DNS 填充”

- 位置：`tools/memory-validation/run.py:169-176`（`--diagnostic` 预填充经由 sender／mock／时序路径）；README:53-55 已承认模板群体 2507 vs 329 不可比。
- 触发条件：任何 BASE/AUDIT 诊断对照（本次已正确地未将其当验收）。
- 实际影响：填充走真实查询路径后，模板/共享应答的组成受时序（缓存命中/未命中、延迟抖动）影响，两个二进制的群体天然不同，15% 审计堆门槛永远缺少因果基线。
- 最小修复：诊断填充改为进程内确定性合成填充（`audit_memory` launcher 已在进程内注入），BASE/AUDIT 用同一份固定合成记录序列，使 unique_templates/unique_responses 逐项相等后再比较；不改产品代码。

### P2-3【K1 修复范围缺口】状态感知方案必须覆盖“测量窗口内在跑的学习”，不只是预存快照老化

- 位置：`rust/native-host/src/route_learner.rs:435-451`（周期写盘，运行中持续学习并更新 `last_seen`）；`tools/memory-validation/run.py:131-137`（preflight 只检查预存快照非空）。
- 触发条件：oracle 表征运行本身也在学习；正式重放使用不同 seed／顺序（501–503），同一名字的第二次出现答案可能依赖第一次出现时的学习状态。
- 实际影响：即便修复了预存条目老化，学习顺序差异仍可能造成 B/C 假阴性 FAIL（或掩盖真差异）。该维度在现有记录中未列出，见第三节修复方案第 3 步。

### P3-4【测量工具】预填充阶段不校验 DNS 正确性

- 位置：`run.py:174-176`，只检查 `total_queries == prefill`。
- 影响：prefill 期间的 wrong 应答只留在 requests.jsonl，不进任何门槛（测量窗口会用同语料复现大部分，但非保证）。最小修复：把 prefill 的 wrong 计数写进 summary 并在正式运行中非零即 FAIL（防御纵深，不改判定阈值）。

### P3-5【测量工具】`--top` 补充语料的“supplementary”标注未强制、未记录

- 位置：`run.py:142-144, 219`；README 第 3 条要求标注，但 `identity.json` 未记录语料来源标志。最小修复：identity 增加 `top_source: default|explicit` 字段，evaluate 对 explicit 附带 supplementary 标签。

### P3-6【候选代码／性能，未验收项】查询路径每查询多一次 `suffix_key` String 分配；miss 查询成本为 O(标签数 × log n)

- 位置：`rust/matcher-core/src/compact.rs:58-69`（suffix 链逐标签二分）；`valued.rs:338`。trie 旧路径首标签未命中即中断。
- 影响：DNS 名长度上界（≤127 标签）使成本有界；库级 `lookup_ns` 三方基本持平（5.12–5.82 µs，`index-probes.json`）。属**未运行的 CPU/p95/p99 正式门槛**覆盖范围，不是缺陷；此处仅登记，勿以库级纳秒推算 DNS QPS。

### P3-7【候选代码／可选优化】`audit_timing_snapshot` 仍在锁内物化整份 Vec（100k/400k × 32 B）

- 位置：`observer.rs:1740-1751`、`api.rs:2701`。与旧实现的 `copied().collect()` 成本相同，**无回归**；但 timing 既已是投影，windows 端点可直接在锁内聚合（免中间 Vec、缩短锁持有）。本轮非必修，不扩范围。

### P3-8【候选代码／信息项】`can_retain` 不再包含 timing 队列预留

- `observer.rs:1436-1439`：旧实现在 timing 队列预留失败时会整条丢弃记录；新实现只看环＋slowest。仅存在于可恢复分配失败路径，语义反而更合理，契约允许“best-effort 丢弃该条记录”。不需处理。

### 无发现（已逐项核验的行为等价）

- **CompactIndex 与 scanner 等价**：`sort_dedup` 的 `dedup_by(|later, earlier| earlier.1 = later.1)` 参数序与稳定排序正确复现 HashMap 插入／trie 覆盖的 last-write-wins；`suffix_key` 先 `normalize`（剥一个尾点）再经 `reverse_labels`（再剥一个），与旧 `add→add_normalized`／`match_normalized` 的双重剥点完全一致，空标签/根域/前导点/双点键在标签边界上等价（`compact.rs:90-111` 对 `DomainSuffixMatcher` 的 oracle 测试覆盖）；键全 ASCII，无 UTF-8 切片 panic 风险；`d7141d59` 的 remap＋`drop(results)` 确实释放被规范化别名替换的 payload，overlap 规则的 Arc 独立保活（`valued.rs:282-296`＋测试 530）。
- **Rank 输出逐字节不变**：按 `IpAddr` 聚合与按 `to_string` 聚合的分桶相同（`canonical_ip` 后字符串表示单射），最终排序仍比较 display 字符串（`api.rs:2338-2343`），tie-break 顺序不变。
- **筛选语义不变**：`audit_record_matches` 候选值集合逐项相同（借用 vs 克隆），client/trace_id 每条仍分配（与旧一致）。
- **契约一致性**：`native-audit-control.md` 要求 "v2 stats and windows derive only from the retained ring"——现在字面成立；clear/resize→timings 复位经环复位实现；逐出计时扣减使用被逐记录的 `elapsed`，与旧 timing 副本数值相同。logs/slowest/TTL/完整应答/Trace ID 文本回退/IPv4 映射 IPv6/双 worker＋499/500 语义（`api.rs:2447-2520`）未被触碰。
- **公开面不变**：matcher-core 公有 API 无变化（`reverse_labels` 仅 `pub(crate)`）；Git 产品零新增依赖（FST 只存在于一次性比较树）。

### 既有／平台问题（本轮不处理，如实保留）

- macOS 11 个失败 target：分类未完成，不得算通过——本审查环境为 Linux，无法复核，维持原状。
- 历史 failure 记录（磁盘耗尽、陈旧 BASE 工件、FST 依赖漂移）保留为证据，未被冒用。

---

## 二、结论

**CODE/TOOL REVIEW: PASS**（仅针对本轮范围：BASE..cbf66577 的候选代码与已运行的 smoke/oracle/证据工具。上述 P2 均为**尚未运行的验收证据的代表性缺口**，不是已产出的证据被证伪，也不是候选代码正确性缺陷；未发现行为回归。PASS 不覆盖：正式三组配对矩阵、审计因果对照、400k/缓存压力、热应用峰值归因、两小时稳定性。）

**MERGE READY: NO**（独立结论，与记录一致：全部正式验收门未完成，K1 未修复。）

---

## 三、K1 最小安全修复／验证方案

**结论核验：成立。** 证据链完整：`prepare.py:27` 拷贝含绝对 `last_seen` 的 `gen/route_learner.json`；harness（`5d-2/harness.py:44`）每次运行 `copytree` 出一次性副本，冻结快照逐次重放；`route_learner.rs:251/311/350` 以 `now >= last + ttl` 过期；oracle 是静态逐名字期望（`5d-1/load.py:76-77` 仅语义相等＋TTL 0..300 边界）；`my_nov6list` 条目 00:51:57 过期、oracle 00:29 生成、00:54 重放，BASE 与组合候选同为 1499/1——确系测量模型失效而非产品回归，判定器与 preflight 阻断（`run.py:131-137`）方向正确。

**最小修复范围（不改产品、不改 TTL、不删问题、不接受任意应答、不事后改阈值）**：

1. **Fixture 时间重基准**（推荐）：在 `prepare.py`（或每次 Env 启动前，仅私有副本）把 `gen/route_learner.json` 全部 `last_seen` 重置为运行起始 epoch（保留相对结构）。过期裕度从“残余存活期”变为整档 ttl（实测档 ≥86400 s）。新增 preflight 断言：`min(各档 ttl) − 预计 oracle↔重放最大间隔 > 安全裕度`（如 2× 最大运行间隔），不满足则继续阻断。这与已做的上游/TLS/审计容量重接线同类，仅动私有副本。
2. **Oracle 逐矩阵再生＋状态指纹**：每套正式矩阵开始前重新生成 oracle，`identity.json` 记录 learner 快照哈希与 oracle 生成/各 role 启动的墙钟；`evaluate.py` 对“距过期不足裕度”的格子**fail-closed**（拒绝该次矩阵并要求重跑），而非静默豁免。
3. **在跑学习确定性对照（P2-3 补充）**：正式矩阵前加一对同 seed 的 BASE-vs-BASE 对照运行，全部格子必须一致；不一致即阻断（说明学习顺序影响答案，需要成对差分模式而非静态 oracle）。
4. **前置条件清单**（恢复正式测量前必须全部满足）：修复经审查合入工具分支；smoke 恢复 1500/1500 全对；每运行记录 learner 指纹；现有 preflight 阻断在 1–3 落地并复审前不得移除。

---

**致主会话的移交要点**：候选代码（`3c065d29`/`d7141d59`/`d6e4b835`）可保留在分支上继续走验收；优先落实 K1 方案第 1、2 步与 P2-2 的确定性填充，P2-1 在正式索引堆门槛采信前补池基数实测；P3-4/5 可随工具下一次改动顺带处理，P3-6/7 留待正式 CPU/p95/p99 结果再议。
