> Historical prompt: this candidate was withdrawn on2026-10-28 after owner-directed attribution work. This is no longer an active review/merge submission. See [the withdrawal record](diagnosis-2026-10-28.md). The original exact-source prompt below is retained for evidence.

# External review prompt

请在 `/Users/tom/github/rtdns` 做一次只读的完整增量审查。先按 `AGENTS.md`
读取项目约定，核对实际源码和证据，不要把本文或测试报告当作结论。
用户为本任务选择人工外部 agent review。不要修改代码、合并 main、push、
发布、部署、重启既有服务，也不要自动联系其他审查会话。

审查分支是 `codex/memory-optimization-validation`。增量基线是
`3687d269b1699b7061a3b48c5e4a9f51745bc625`，产品冻结头是
`fb61c2474e663e39c88e1aa55831eb8006731bfb`，测试方法冻结头是
`7b72bba7f330c60aca1005cc77be0c78981a78cd`，独立读取矩阵方法头是
`cbff46d20f64cb24b15e09174269ee30db741f75`。开始时记录实际 HEAD，并确认
`fb61c247..实际HEAD` 没有额外 Rust 产品源码变化。最终证据范围请连同同目录 README、evidence.json
一起审查。旧内存阶段的验收、review 和 CPU 例外不代表本增量已通过。

本任务只做三个无损改动，不能以增加取舍来修正问题：

1. `a9c4447a`：缓存命中在 lookup 已经拥有的 TTL 老化 Vec 上原地修改 ID/RA，
   省去第二份 wire；缓存原件、TTL/lazy、origin/domain_set/ECS 和错误行为保持。
2. `93410152`：四种分组排行准确选择 top-K 后才生成展示标签，仍对全部域名
   计算 distinct；独立 slowest300、过滤、排序、两 worker 和取消约束保持。
3. `fb61c247`：cache show/dump/save 采用不可变共享条目快照，保留旧 owned
   API、完整应答搜索、分页/显示/错误行为、导出预算、v2 编码及持久事务。

重点核对以下风险：

- S1 的可变缓冲是否确实归本次请求独有；先验证后写；ID/RA 之外字节不变；
  连续不同 ID、fresh/lazy TTL 和原缓存 wire 是否保持正确。
- S2 的分组 equality/Ord 与 API 展示 tie 顺序有区别：原始域名、ASCII 大小写、
  canonical IP 与 IP 文本比较；heap 的 worst-first 和最终排序是否准确。
  K=0、小 K、极大 K 的输出与全量 distinct 是否独立；内存仍有 O(N) 输入键，
  不能误称全部排行空间 O(K)。取消及 worker/snapshot 生命周期是否保持。
- S3 是否只共享不可变数据；capture-time 过期判定、双时钟、wall_times 和
  attachment/provenance 是否自洽；replacement/flush/import 后旧视图一致、
  最后引用释放。logical bytes/entries 限制不能因共享放大；v2 framing/CRC、
  全量验证后导入、原子 save/flush/失败留旧状态不能变。
  show 必须仍验证每个原先会访问的条目；完整 answer 搜索和 offset/limit
  不能因延迟格式化丢失或改变结果。
- 验证标准构建的247/248个输入是否对应上述 Git 源码，分配探针是否与普通
  timing binary 分开。源码与工具指纹、profile 机械派生、配对顺序、种子、
  覆盖率、保留环、协议失败、语义差异、读取峰值和清理证据是否完整。
  单个模型或 allocator 请求字节不能替代 RSS；读峰值不能冒充热应用峰值。
- 第一轮 `observations-v1` 因4个 sender unsent 停止，不能重新标 PASS。
  V2 只改变发送器 nice 优先级、保持 sampler absolute nice+10，并保持全部
  覆盖阈值；这不证明原调度空档的根因。核对是否确实如此，以及是否有
  选择性报告、缺批次却标通过或沿用旧阶段豁免。
  后续 `--collect-incomplete-sends` 只允许继续收集，不放宽原 summary 的资格
  判定；单独的 unqualified_descriptive_metrics 不得被误称验收通过。以漏发为
  理由继续的角色若含未知错误、timeout或orphan必须被拒绝；原有输入/清理
  门槛保持不变。独立闭环读取矩阵和发送器诊断
  不能替代三组 DNS 性能资格。
  本次完整收集已有71个候选unsent，原资格为incomplete；角色中位数描述值
  CPU -2.25%、p95 +5.81%、p99 +16.82%。不得改用配对百分比中位数来洗掉
  延迟超标，也不得在缺因果证据时直接断定是某个代码改动导致。
- macOS 断连后第三次读取应503却200，在独立原始3687d269上也复现；Linux
  全工作区1541/0/0包含该测试。请核对基线证据和本次是否另有新风险，不能
  仅因写了“已有问题”就忽略，也不能直接把同一旧失败认作本次回归。

先读 `README.md` 和 `evidence.json`；诊断工具说明在
`tools/memory-validation/LOSSLESS-READS.md`。私有原始证据在
`ssh mosdns-rust:/root/rtdns-lossless-20261027`：标准构建/输入 manifest、
CI 日志、probe、v1失败、v2数据、读取响应和真实浏览器操作记录。
只读检查即可。不要连接生产 `mos`，不要将配置值、查询原文或凭据写入公开报告。
不要求为了 review 重跑长测；发现可疑处先做最小、安全的复现。

请输出可执行的问题清单，按 P1/P2/P3 排序，每项包含准确文件/行号、触发
条件、实际影响、当前证据及最小修正方向。若没有可确认问题，请明确说明。
分别给出“代码审查结论”和“验证/验收是否完整”，不要把无代码问题当成
全部性能/生产门槛通过。存在噪声、缺失批次或未执行项目必须保留为未完成。
