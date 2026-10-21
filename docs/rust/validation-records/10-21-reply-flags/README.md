# 本地应答 RD/CD + 并发上游选取对照（M2-4 第 3、4 项）

验证日期：2026-10-21（Asia/Taipei）。基线 `e8b4425b`，分支 main，未提交。外部审查见任务 `research/codex-review-round-*.md`。

## 结论

- **A（已修）**：Rust 本地构造的应答（REFUSED/SERVFAIL、reject、hosts、rewrite、ECS/redirect 包裹的本地应答、预算 SERVFAIL）此前 RD/CD 恒为 0。
  现在 `QueryHeader` 保存查询的 RD/CD，唯一构造点 `synthesize_response` 写入应答，QR/RA 不变、AD 为 0。上游与缓存应答不动。
- **B（已对齐，未改行为）**：并发上游的分级（A/AAAA 应答 > 首个 NOERROR/NXDOMAIN > 首个其它 rcode > 首个错误）、同级先到者、
  地址应答取消其余路、超时语义与 Go 一致；唯一会在默认配置下改变结果的差异：一路静默到请求截止、另一路已回 NODATA/NXDOMAIN 时，Go `forward` 回 SERVFAIL，Rust（及 Go `aliapi`）回已收到的应答；
  Rust 侧更优，**未改；用户已决定（2026-10-21）保持 Rust 现状**。另有 `concurrent > 上游数` 时 Go 重复查询同一上游，不改变结果（逐条见任务 `research/go-semantics.md`）。
  Shadow-1 token 20061 / Shadow-2 token 198 的 SERVFAIL 因此不是选取逻辑造成，需要靠新增证据归因。
- **证据**：诊断抓取运行时，每次上游尝试多一行 `dns upstream attempt`（`trace_id`、`domain`、`qtype`、`entry_index`、`upstream`、
  `outcome`，应答时另有完整 `rcode`、`has_address`），仅追踪中的查询写；审计持久格式与 API/WebUI 字段不变。

## 验证统计

| 验证 | 结果 |
| --- | --- |
| 修复前 `reply_flags` | 首个场景 RD 不一致而失败（只记录了第一处失败） |
| 最终 `reply_flags`（6 类 × UDP/TCP × 4 种 RD/CD） | 2 个测试通过 |
| 选取矩阵 `upstream_selection` | 7 通过（行为未改，首次即通过；含“一路静默到请求截止”） |
| trace 行端到端（`f2_diagnostics` 真实二进制） | 通过 |
| dns-core / udp_response / assembly 新单测 | 各 1，通过 |
| VM fmt / 严格 clippy | 均退出 0 |
| VM workspace 全量（含 doctest） | 1531 通过、0 失败、1 ignored；净增测试 12 |
| 全量过程中的一次波动 | `f3a_update` 一个 URL 更新测试超时一次，最终树上重跑通过，与本任务无关 |
| 真实上游 / 生产机访问 | 0 |

未做：分支路径（fallback/ECS/redirect）的 trace 行端到端用例、rewrite IP 目标与 TTL 物化的端到端 RD/CD 用例、与 Go 的逐字节应答比对。

契约：`native-audit-control.md`（“Locally constructed reply flags”）、`native-webui-hosting.md`（“Upstream attempt line”）。
