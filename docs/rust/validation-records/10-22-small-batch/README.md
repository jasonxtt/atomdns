# Small batch：ip_set、macOS、纯后端说明、Shadow-2 诊断

2026-10-22，Asia/Taipei。BASE `701a5d9fd0fe2210f16dd691d5639325dc41f336`。
实现与执行验证已完成。外部 Claude 第 1 轮因 V1 验证缺口判 FAIL，代码阅读未发现 P1；
补充结果见文末，等待第 2 轮审查，本记录不代表审查通过。未提交、发布或部署。

## Shadow-2：名字 A 的负缓存差异

**判定：证据不足以确认 Rust 缺陷，也不足以把原差异改为已解释。**
最可能的机制是两端对同一名字持有不同时间取得的空 NOERROR/NXDOMAIN 缓存；
新证据显著支持 Go 侧返回的同一份旧负应答持续老化这一部分，无法定位 Go 宿主/上游的缓存层，
也无法还原 Rust 缓存条目的原始来源或刷新结果。
没有修改缓存、序列或 DNS 应答代码。

### 可直接核对的事实

只读分析本机仓库外 `final-private.tar.gz`，SHA256 为
`d2a796e555545217c7cb21ec6ef3a26d5df79eb6d4e60f8e65d73c33e524bb61`，
与 Shadow-2 公开记录一致。解析完整保留的差异 wire、audit 和 trace，未发送 DNS、未连接生产或已关机影子机。

| 观察 | 结果 |
| --- | --- |
| NXDOMAIN→NOERROR，仅 rcode 的目标行 | 214；同一名字 A、同一客户端，全部为 A 查询 |
| 时间（+08） | 2026-10-21 00:43:52.686–00:51:53.828 |
| Go 响应 | 214 次均为 NXDOMAIN，Answer 空，Authority 有 SOA，另有 OPT；140 bytes |
| Go SOA TTL | 随时间从 483 降到 1；每行 `sent_wall + SOA TTL` 落在 00:51:52.829–00:51:55.777，跨度 2.948 秒 |
| Rust 响应 | 214 次均为 NOERROR，Answer/Authority/Additional 全空；41 bytes，没有可观察 TTL |
| Rust 新 attempt | 214 次全部为 0 |
| Rust trace | 214 次都经过同一个缓存 C，之后只有 tag_setter 和 exit，再完成；缓存之前的 matcher 及真假结果序列全部相同 |
| 同名其它保留差异 | 16 行反向 NOERROR→NXDOMAIN，8 行 AAAA、8 行 A；分别在 02:05:43.300 起和 02:22:02.224 起出现，末行 02:22:22.257 |
| 反向差异的 wire | Go 空 NOERROR（仅 OPT，52 bytes）；Rust NXDOMAIN 带 SOA（103 bytes），无新 attempt，同样经过缓存 C |
| 学习采样 | 14 次；8 个学习文件的首尾计数一致，无下降，沿用原记录的非零计数 5435、1260、135 |

分析只公开计数、时间、DNS 类型和 TTL；名字、客户端、供应商、代理及其它原始标识均未复制到仓库。
SOA 内容未公开；表中到期区间是 wire 内 SOA TTL 的推算归零时间，以客户端提交时间估算，
包含 TTL 秒级舍入和两端时间/请求处理偏差。缓存实现可另设保留时间，不能据此认定缓存条目的实际到期点。

### 推断与证据边界

- Go SOA 的递减曲线与几乎固定的归零时刻相符，支持这 8 分钟集中差异反复返回一份老化负应答，
  最可能来自已有缓存。没有 Go 查询级 trace，无法区分 Go 宿主缓存和其上游缓存，
  也不能把它提升为“214 次均已证明没有重新访问上游”的事实。
- Rust trace 的缓存后路径与无 attempt 支持缓存命中。后半段同名出现反向差异，且 Rust 返回带 SOA 的 NXDOMAIN，
  与“两端空应答/负缓存取得时间不同，之后各自更新”相容；它反对“Rust 对名字 A 永久改写 rcode”的简单解释，
  但不能排除其它 Rust 缺陷。
- 当时源提交 `bc957a24` 的 `cache.rs` 允许缓存空 NOERROR，并在配置 lazy TTL 时延长其保留；
  `retention_seconds` 对 NXDOMAIN、空 NOERROR 和有记录 NOERROR 分别处理。
  因此不同旧条目、惰性返回/后台刷新时序是可行机制，**不是已经还原的事实**。
- 归档只保存非一致/不完整行的完整 wire。数值帧不带名字，不能从中找出名字 A 的窗口前首次应答、
  中间一致应答或每次刷新。归档没有起始/结束缓存 dump、完整上游 wire、Go 查询级 trace 或完整历史配置快照。
- 学习计数不变不等于逐条名单内容相同；matcher 结果一致只证明保留的 214 行走了同一路径，
  无法还原其它请求的学习行为。run 的事件计划为空，但缺少可独立重建全部开关变化的原始快照。

若后续要确认因果，需要在另行授权的观察中同时保留两端同键缓存条目的来源/过期时间、
冷 miss 和后台刷新应答，并连续记录该名字的全部请求；本任务没有补采真实流量。
原 Shadow-2 的 249 条未解释及其 214 行子类保持原计数，不把推断当作豁免。

## 实现与验证状态

### A. IP 管理

真实 config_rust 的 direct_ip/client_ip 均为单个 `.txt`、无内联 IP。按用户确认的规则实现：
所有 ip_set 均可 show；post 仅允许单个独占 `.txt`、无内联 IP，其它形状返回 400 和具体原因。
共享检查覆盖规范实际路径、现有符号链接/硬链接、跨类型与匿名文件引用、禁用规则源及运行中新增的清单项。

复用现有 IP handle、构建线程、文件锁与原子替换；成功后 client_ip/resp_ip 都读新快照，重启保留。
post 的管理租约与热应用提交前指纹核对配合，旧候选返回 409；写入失败不改文件或内存。
show 和 N 使用排序、规范化、折叠后的前缀，符合 Go Sort；不添加 save/flush。
能力声明只扩展既有 `rules.local.manage` 的可用条件，不新增字段或 API level。

### B/C. macOS 与纯后端说明

macOS 修复仅按平台限定 Linux rustix 调用，以及非 Linux 不使用的 UI/metrics helper；
未改 Linux 分支执行语句、依赖或原有“不支持在线更新”报告。没有在本机运行测试。

新增 [纯后端说明](../../headless.md)，README 链接到它：持久设置位置、代理/上游/ECS 三个 JSON 示例、
YAML 默认值与 EFFECTIVE 来源、check 后重启或 API 热应用，以及专属组 JSON 权威关系。
三个示例分别、合并后及 YAML 默认共 5 个 case 均通过 VM 真实 `rtdns check`；没有连接示例上游。

### 验证与清理

定向测试已通过：HTTP/DNS/重启与配置形状 4/0，确定性 post/热应用 409 测试 1/0，持久化故障测试 1/0。
第 1 轮送审源码的全量结果（P2 补充后的重跑见文末）：

| 检查 | 结果 |
| --- | --- |
| VM `cargo fmt --check` | PASS |
| VM `cargo clippy --workspace --all-targets -j2 -- -D warnings` | PASS |
| VM `cargo test --workspace --no-fail-fast -j2` | 1546 passed / 0 failed / 1 ignored |
| macOS `cargo check --workspace --all-targets --offline -j2` | PASS |
| macOS `cargo clippy --workspace --all-targets --offline -j2 -- -D warnings` | PASS |

基线原有测试 1540 passed / 1 ignored，新增 6 个（2 unit、4 集成），忽略数未增加。
VM 离线、CARGO_INCREMENTAL=0、-j2，独立 target；关闭 dev/test 调试信息节省磁盘。
初次 tmpfs 空间不足后改用系统盘；测试 fixture 修正后在旧源码实际复现 show 404，再验证新实现。
rsync 时间戳误用旧 matcher 产物的构建失败已通过 touch/rebuild 修正；最终全量是在最后源码上重跑，16 个修改/新增 Rust 源文件本机/VM hash 一致。

最终测试日志 SHA256：`c6cb2bc4587bfd5df322fd975cc4c6a75c0124d6d423e604b328acaaef4904a4`。
含基线、RED、定向、最终全量及文档 fixture 的日志归档 SHA256：
`69d40ab86fa7a46d3fe3299db317599d486c73b1db9619346a8a42f85cf6585a`（本机仓库外保存）。

清理确认：VM 本任务进程 0，构建/测试临时目录已删除；原 mosdns 服务仍 active、PID 827437、NRestarts 1，与开工一致。
Shadow-2 的本次私有解压目录、脚本和中间结果已删除，原归档保留且 hash 相同。
未连接生产或已关机影子机，未修改 Go/file 仓库、未自行审查、未提交/push、未发布或部署；等待外部审查。

## Claude 第 1 轮补充：V1 与 P2

V1 要求在真实配置验证新增编译输入记录/文件碰撞检查，以及独占判断是否误拒生产使用形状。
BASE 与候选均重新用标准构建脚本构建，UI 先于 Rust，VM 离线、2 jobs、禁用 incremental。
只读复制真实配置包；生产快照按 M2-3 相同 prepare 流程生成 native 根，运行只用 disposable 副本。

| `rtdns check` 输入 | BASE/candidate exit | stdout bytes | stderr bytes | 直接 byte 比较 |
| --- | --- | --- | --- | --- |
| 真实 config_rust 副本 | 0 / 0 | 1377 / 1377 | 160 / 160 | 两流完全一致 |
| 快照转换的 native 根 | 0 / 0 | 1484 / 1484 | 0 / 0 | 两流完全一致 |

没有差异需要解释；原配置包、原快照的内容清单在验证前后相同。
启动隔离采用既有 5D-2 harness：1 个回环接口、0 默认路由，不连接生产或公网；mock 只返回 RFC 文档地址。

| 真实 tag | 文件有效/折叠前缀数 | show | post | RFC 替换后前缀数 | show 与文件 |
| --- | --- | --- | --- | --- | --- |
| direct_ip | 1 / 1 | 200 | 200 | 2 | 完全一致 |
| client_ip | 8 / 8 | 200 | 200 | 1 | 完全一致 |

有效前缀独立解析、掩码、排序和删除被包含项后与完整 show 正文相同；两个 tag 均没有误报共享。
capabilities 返回 200，既有 `rules.local.manage` 为 true。
在另一个快照副本通过现有开关 API 选择观察路径、关闭缓存，用 RFC 源地址与示例域名发起 4 次 UDP 查询：
两个 tag 的 client_ip/resp_ip matcher 各从 false 变为 true，全部 rcode 0、1 条 RFC Answer，证实替换即时生效。

P2 已处理：超过 IP 条目数或输入字节上限返回 400，持久化/构建失败仍为 500；新增 1 个测试分别触发两种上限，
确认文件和 live 快照均不变、无临时文件残留。正常 HTTP 正文总上限的 413 行为保持。
backlog 的规划方待办没有在本轮修改。

P2 最终源码重跑：VM fmt、workspace/all-targets 严格 Clippy、workspace test 均 exit 0；
**1547 passed / 0 failed / 1 ignored**，96 个结果组，比首轮增加 1，比原基线增加 7。
macOS workspace/all-targets check 与严格 Clippy 均通过，未运行本机测试。
16 个 Rust 文件本机/VM hash 全部匹配；源码清单摘要
`756b478fd9bc29db09a37b67f449942f2721e9bb2f56c15fd716f6089f4fd8b5`。
当前全量测试日志 SHA256 `75f379f8617799043190c144356a6ac42326cdec9ec8c2fbb393e05509e62d97`；
本轮私有证据归档 SHA256 `0f564349721c7ec01ad88cf36b08f9c45d15a1c09d20644ca5297308951ce1e0`。

本轮 VM 临时目录已删除、所属进程 0、全部隔离 netns 已删除；原服务 active/PID 827437/NRestarts 1 不变。
本机工作副本与脚本已删除，私有证据归档留在仓库外。没有提交、发布或部署，等待 Claude 第 2 轮结论。
