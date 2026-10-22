# M2 规划（首次生产替换之后）

状态：规划草案（2026-10-17，用户同意记入），**不阻塞首次替换**，不授权实施。M1（`config_rust` 原生运行）与 5D/Phase 6 完成并切换后再排期。
每项开工前按现有流程写 PRD、冻结设计，经用户确认。

## M2-1：具名、带类型的功能开关（取代 `switch1`–`switch17`）

### 现状问题
- 17 个开关只有编号，含义只写在注释与 WebUI 代码里（如 `switch3` 泄露/不泄露、`switch4` 过期缓存 1、`switch5` 屏蔽 SOA/PTR/HTTPS、`switch6` 屏蔽 AAAA）；
  值只有 `'A'`/`'B'`，存于 `rule/switchN.txt`；`config_rust` 中约 84 处 `matches: switchN 'X'`。
- WebUI 需写死“编号 ↔ 按钮 ↔ A/B 文案”；新增开关要同时改配置与前端并挑空编号。
- 开关与 `fast_mark` 共享位命名空间（`config-notes`：`switch17` 占 bit 49、`fast_mark 48`），撞位会导致路由错误。
- 为一个开关复制整段流程（如 `cache_all` / `cache_all_noleak` 两段由 `switch3` 二选一），组合越多配置越长；多选一只能拆成多个开关。

### 方案
- 新增配置对象 `feature_flag`：`label`、`description`、`kind`（`bool` 或 `options` 枚举）、`default`；流程中按名引用（如 `matches: flag leak_mode=noleak`），
  加载期校验名称与取值。
- WebUI 读取声明自动渲染开关页（说明、选项、“已修改/恢复默认”），新增开关不改前端。
- 能用参数表达的整功能启停改为参数绑定开关（如缓存 `enabled: $cache_enabled`，关闭即直通），合并按开关复制的流程段。
- 开关值独立存储，不占 `fast_mark` 位；值存入 `webinfo/settings.json`（M1-f1 设置层，只存差异）、`check` 与只读 API 显示生效值与来源、热生效、
  每条查询准入时取快照。

### 步骤
1. 盘点 17 个开关的真实含义、选项、默认值、UI 文案与配置引用，形成对照表。
2. 实现 `feature_flag`，改写 `config_rust`；迁移工具把现有 `rule/switchN.txt` 的值转换为具名开关的设置（用户设置不丢失）；用 E2E 差分证明路由不变。
3. WebUI 开关页改为按声明自动生成；Go shell 不变。

## M2-2：域名“自学习”的重新设计（`domain_output` + `requery`）

### 现在实现的功能
根据上游应答**学习每个域名的路由事实**，下次直接按记忆分流，减少探测、泄露与无效等待：
- `my_realiplist`：国内 DNS 返回国内 IP → “记忆直连”（mark 11）；
- `my_fakeiplist`：应走代理 → “记忆代理”（mark 12）；
- `my_nov4list` / `my_nov6list`（及节点类）：rcode 0/3 但无 A/AAAA → “记忆无 V4/V6”（mark 2/3），后续直接空答；
- `my_sv4list`：国内无解析、国外有解析的特殊 v4 域名（mark 4）；
- `top_domains`：访问排行与计数，供 WebUI 排行与 `requery` 取源。
生产规模（2026-10-06 快照）：realip 约 5,030、fakeip 约 2,093、nov6 约 3,109、nov4 约 586、sv4 7、top 约 10,486。

### 现在的实现手段
1. 学习条件以 `matches + exec: $my_xxxlist` 的形式**分散写在多个流程**中（`process_v4`/`process_v6`/`forward_1` 等多处重复）。
2. 每条学习结果经过多层管道：内存统计 → `file_stat` 统计文件 → `file_rule` 规则文件 → 经 HTTP 回环 POST 给对应 `domain_set_light`（整组替换并写同一规则文件）
   → 触发 `unified_matcher1`（约 14.8 万条规则）**整体重建**；同时 `QuickAdd` 写入 mapper 热表以便立即生效（Go 在整体重建时清空热表）。
3. 定时写盘间隔约 10 小时（`dump_interval` 36000+ 秒）。
4. 没有逐条过期：陈旧问题靠 `requery` **每 3 天清空全部学习列表与缓存**，再按 `top_domains` 近 30 天的域名以 10 qps 重新查询“重新学一遍”。
5. 学到的域名保留查询原始大小写（如 0x20 大小写随机化或客户端大小写），写成 `full:` 规则；Go 中大写规则实际不生效，
   原生按已批准的大小写不敏感使其生效（即 5D-1 中 462 个单元格差异的来源）。

### 评价
- **目标合理，是本 fork 的核心价值之一**：把“探测一次的结论”变成后续查询的快速路径，减少泄露与等待。
- **手段问题**：
  - 同一事实存在多处（内存统计、统计文件、规则文件、domain_set_light、mapper 热表），且规则文件有两个写入者（domain_output 与 domain_set_light 的 POST），需要额外时序围栏；
  - HTTP 回环与整张 mapper 重建只为加入少量新域名，代价与收益不成比例；
  - 学习逻辑散落在流程中且重复，改一处容易漏另一处；
  - 全局清空 + 重学：清空后一段时间内学习成果全失，重学期间产生集中查询，近 30 天排行之外的知识直接丢失；
  - 负向学习（无 V4/V6）没有单独的证据门槛与时效：一次临时的空应答就可能让该域名长期拿不到 IPv6，直到下一次全局清空；
  - 10 小时写盘间隔，非正常退出可能丢失大量学习结果；`max_entries` 是提前写盘触发而非容量上限；
  - 域名未规范化（大小写、尾点），导致 Go 下出现“写了却不生效”的规则。

### 改进方案
1. **统一的学习存储（单一事实来源）**：每个类别一个原生 owner，键为规范化域名（小写、无尾点），值含结论、首次/最近确认时间、命中次数、证据（哪个上游、IP 类别）。
   mapper 通过**独立的学习覆盖层**查询（优先级明确），不再经 HTTP 回环、不再触发整张 mapper 重建、不再写重复规则文件。
2. **逐条时效与按需复核**：每条记录有 TTL（按类别设定，如正向 7 天、负向 1 天），过期淘汰；使用到较旧的记录时在后台按小概率/按年龄复核（类似缓存 lazy 刷新），
   结论不符即更新或撤销。取代“每 3 天全局清空 + 重学”，`requery` 仅保留为可选的主动预热。
3. **负向学习更谨慎**：要求连续多次一致观察或更短时效；SERVFAIL/超时不学习（现已如此）。
4. **声明式学习规则**：如 `type: route_learner`，参数含类别、条件（上游组、`resp_ip` 集合、qtype、rcode）、mark、标签、TTL；由主机在应答阶段统一评估，
   取代分散在各流程中的 `matches + exec` 重复块。
5. **有界与持久**：真正的容量上限（按最近确认时间淘汰）；追加式日志 + 定期压实，非正常退出只丢秒级数据。
6. **可见与可控**：WebUI 列出学习条目（结论、原因、年龄、命中），可单条固定/删除/导出；指标含条目数、命中、过期、复核结果。
7. **迁移**：从现有 `gen/*list.txt` / `gen/*rule.txt` 导入（规范化大小写），保持“记忆直连/记忆代理/记忆无V4/V6”等标签与 mark 语义，用 E2E 差分证明分流结果一致。

### 可提前的小项（可在 5D-2 或 Phase 6 期间评估）
- 学习写入时规范化为小写（不改其它结构），消除“大小写不同的重复/无效规则”这一类问题。需单独评估对现有生成文件与 Go 回退的影响后再决定。

## M2-3：热应用编译移出 DNS 线程

5D-2b（2026-10-17）测得：设置 API 修改与专用组保存的热应用在唯一 DNS 线程上编译配置，DNS 停顿约 0.5–0.65 s；500 qps 下内核 UDP 缓冲溢出丢包，
生产负载（数十 qps）下仅表现为约 1% 查询延迟。首次替换前用 5D-2c 增大 `SO_RCVBUF` 并写明运维说明。根治方案：编译在构建线程完成后把结果交回 DNS 线程安装，
需要把编译结果中的单线程类型（`Rc`/`RefCell`）改为可跨线程或在 DNS 线程做最小化的“装配”步骤。首次替换后评估。

**2026-10-22 已完成（任务 `10-22-rust-native-offthread-apply`）**：候选编译改在一次性编译线程上执行，join 后交回 DNS 线程（`compile_thread.rs`，用户批准的一处 `unsafe impl Send`），
编译串行、关机放弃、panic 为无效变更，事务语义不变。用户决定不追求延迟目标：发布与旧代退役（`domain_output` 交接存盘/读盘、旧代释放）仍在 DNS 线程上，
实测在 500 qps 下仍可让个别应答延后数十毫秒（数字见 [10-22-offthread-apply](../validation-records/10-22-offthread-apply/README.md)），继续移出它们是可选的后续项（设计 3.2/3.3）。

## M2-4：切换生产后的待办（2026-10-21 记录）

生产已于 2026-10-21 由用户手动切换为 rtdns（rust-v0.1.4，独立服务 `rtdns.service`、配置根 `/cus/rtdns`，Go 原样保留可回滚）。

**随 UI 重做一起处理（用户决定 2026-10-21）**
1. **本地规则页隐藏固定名单**：`webui-log/src/components/ListManager.vue` 中原生模式下 `special_groups.enabled` 为真时只列专属分流组名单，
   白/黑/灰名单、rewrite、ddns 等固定名单不显示（6b 退役托管模式后的残留判断）。规则本身由后端加载生效，`/plugins/<tag>/show` 正常。
   排查其它页面同类 `special_groups.enabled` 残留。
2. **`ip_set` 缺 `/plugins/<tag>/show` 与 `/post`**（后端）：`client_ip`、`direct_ip` 返回 404，Go `plugin/data_provider/ip_set/ip_set.go` 有此接口。
   名单从文件加载并生效，但无法经 API/WebUI 查看或修改；新 UI 需要此后端接口。
   **2026-10-22 已完成**：新增 show/post；post 仅允许单个独占 `.txt`、无内联 IP，复用原子写与活快照；旧热应用候选冲突返回 409。
   验证与 Shadow-2 诊断见 [10-22-small-batch](../validation-records/10-22-small-batch/README.md)。

**真实流量（Shadow-1/2）留下的线索**
3. **并发上游的应答优先级**（P2）：Go `aliapi` 等待并发各路，优先“含 A/AAAA 的应答”→ 首个 NOERROR/NXDOMAIN → 其它 rcode；
   疑似 Rust 会采用先到的 SERVFAIL（Shadow-1 token 20061、Shadow-2 token 198，同为 `local_addlist_exit_v6`→domestic）。先诊断再修。
   **2026-10-21 已查证（任务 10-21-rust-native-reply-flags）**：分级、同级取舍、超时/取消语义与 Go 一致，未改行为；已补逐路上游 trace 行 `dns upstream attempt`（rcode、has_address），见 `docs/rust/validation-records/10-21-reply-flags/`。
4. **本地合成应答不复制 RD/CD**（P2）：Go `SetReply` 复制 RD；Rust `protocol_error`/`synthesize_response` 为 RD=0。
   **2026-10-21 已修**：`QueryHeader` 带 RD/CD，`synthesize_response` 写入；见同一验证记录。
5. **热应用后内存台阶**（P2）：整代热应用时峰值约 176 MiB、之后 RSS 停在约 118 MiB（新进程约 51 MiB），稳态贴近 128 MiB 预算；与 M2-3 一并评估。
   **2026-10-22 已修（同一任务）**：原因有二——启动代被装配对象、管理监听与启动监听常驻引用（第一次热应用后整代不释放），以及候选编译在主 arena 里与缓存交错造成的碎片。
   解除常驻引用并把编译移到一次性线程后，6 次热应用后的 RSS 为同流量不热应用对照的 1.07–1.10 倍（之前 1.72），应用期间峰值 150–152 MiB（之前约 168）。
6. **Rust-only TC 偏多**（P2，原因未明）：Rust 应答 additional A 更多；“Rust 扩大上游 OPT”的假设已被否定（Go 反而统一加 OPT 1200）。
7. **上游查询形状**（待定契约）：Go 对上游统一 OPT 1200、DO=0；Rust 普通转发原样、ECS 时 1232。是否对齐需先定契约。

**Shadow-2 的 214 条 NXDOMAIN→NOERROR**（2026-10-22 诊断）：Go SOA TTL 483→1，推算到期区间跨度 2.948 秒；Rust 全为空 NOERROR、无新 attempt。
同一名字后半段另有 16 条反向差异。最可能是两端缓存取得/刷新时序不同，但缺少初始缓存与首次上游应答，判定为**证据不足**，保持原未解释计数、不改代码。
数值、推断和限制见上述验证记录。

**独立为 rtdns 仓库后（2026-10-21 记录）**
8. **项目改名收尾**（2026-10-22 任务 `10-22-rtdns-rename-release`：已完成；2026-10-22 发布 v0.2.0，生产手动替换为 v0.2.0 后经 WebUI 在线更新到 v0.2.1）：
   用户决定不做桥接版（旧仓库从未发布 `rust-v*`，Go 更新器有一条路径读 `/releases/latest`）；tag 为 `vX.Y.Z`，发布包 `rtdns-linux-amd64.tar.gz`/`rtdns-update-manifest.json`，
   在线更新只来自 `jasonxtt/rtdns`，cargo bin 改为 `rtdns`，`native-host` 版本 `0.2.0`；README 已重写，发版与手动替换见 [release.md](../release.md)，
   演练记录见 [10-22-rtdns-rename](../validation-records/10-22-rtdns-rename/README.md)。生产 `rust-v0.1.4` 由用户手动替换为 `v0.2.0`。
   仍待决定：WebUI 里的 “MosDNS” 界面文案（待 UI 重做时一并处理）；`scripts/cutover/`、`tools/cutover/` 仍按旧名/`rust-v*` 编写，已不适用于新程序。
