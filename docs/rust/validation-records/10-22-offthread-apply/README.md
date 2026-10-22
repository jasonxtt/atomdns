# 10-22 M2-3：热应用编译移出 DNS 线程，解除启动代常驻引用

任务 `10-22-rust-native-offthread-apply`（第二段），基线 `ff885307`。第一段的测量与设计在任务的 `research/measurements.md`、`research/design.md`（本地，不进仓库）；
用户在第一段之后决定：只做“解除启动代常驻引用”（设计 3.1）和“一次性编译线程”（方案 C），不追求延迟目标，验收只看内存与正确性。

构建、测试、测量全部在 VM `mosdns-rust`（Linux `7.0.9-x64v3-xanmod1`，2 vCPU，glibc 2.41，rustc 1.95.0）上进行，本机不构建 Rust。
测量输入按 5D-2b 流水线重新准备（仓库外的生产快照 → Go 参考预言 → 原生预言）；本记录只有数字与方法，不含快照中的域名、地址或上游。

## 改动

| 文件 | 内容 |
| --- | --- |
| `rust/native-host/src/compile_thread.rs`（新） | `compile_off_thread`：在新建线程上运行候选编译，在 `spawn_blocking` 里 join 后把结果交回。结果是单线程图（`Rc`/`RefCell`），用私有包装 `Joined<T>` 的 `unsafe impl Send` 移交——本 crate 中 `memory.rs` 之外唯一的 `unsafe`，用户已批准；健全性论证写在模块注释里：闭包输入是 `Send`、`static` 不能放 `Rc`、编译线程的 thread-local 在 join 返回前已析构，所以移交时值里所有引用计数的持有者都在值内部。对外只接受编译结果类型（不含锁守卫等线程绑定状态） |
| `runtime_snapshot.rs` | `apply_settings`/`apply_groups` 经 `compile_candidate` 编译：异步门串行化（同一时刻最多一个候选在编译）、与关机 `select!`（关机 → `ApplyFailure::Shutdown`，不写任何文件）、编译线程 panic → `ApplyFailure::Invalid`；编译期间若有别的变更发布（代号变了），候选以 busy（409）拒绝，避免“旧视图里的设置/专属组 + 发布后才读到的文件指纹”通过提交点检查（Codex 第 1 轮 P1）；`apply_groups` 在编译前放下当前视图。其余事务流程（prepare、管理排空、提交点、发布、退役）不变 |
| `assembly.rs`、`main.rs` | `HostAssembly::run(self)`：所有监听绑定完成后放下装配对象自己持有的启动代（`config`/`forwards`/`cache` 与 `options.refresh_environment`）；运行时在 `run` 返回前结束 |
| `api.rs` | 管理监听在挂上 `RuntimeControl` 后不再持有启动代的配置与缓存（每个请求本来就用当前视图）；`/api/v1/system/health` 的配置根取自当前视图 |
| `udp.rs`、`tcp.rs` | 只有不受监督的监听（测试与嵌入用的 `bind`/`run_udp`/`run_tcp`）持有上游与缓存对象并在停止时关闭它们；受监督的监听（启动绑定的与热应用新增的）不再持有任何一代 |
| 契约 | `native-settings.md`（Hot apply）、`native-special-groups.md`（Save pause）、`native-webui-hosting.md`（UDP receive buffer and apply pause）：编译不再在 DNS 线程上；发布与旧代退役仍在 DNS 线程上，可能让应答短暂延后 |
| `tools/5d-2/` | 场景 `H0`（同负载无操作）、`M0`/`M1`/`M3`（6 次热应用 + 空闲 300 s，`/proc` 内存快照与 VmHWM 重置） |

## 测试

| 命令（VM，`rust/`） | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | 通过 |
| `cargo test --locked --workspace --no-fail-fast` | 1540 passed，0 failed，1 ignored（基线 `ff885307` 同一 VM：1532 passed，1 ignored；新增 8 项） |

新增测试：`compile_thread`（单线程图完整交回、在独立线程上运行、panic → 错误、放弃等待时结果恰好释放一次）；`runtime_snapshot`：
`hot_apply_frees_the_startup_generation_under_run`（走生产 `run` 路径做一次设置热应用后，用 `Weak` 断言启动代的配置、上游与缓存目录都已释放；
把 `api.rs` 中放下启动代的一行改回去后该测试失败：配置与缓存仍存活）、`concurrent_settings_saves_publish_exactly_one`（两个设置保存同时发出：恰好一个发布，另一个 busy 或 409，文件与生效设置等于发布的那个）、
`compile_shutdown_and_panic_change_nothing`（编译 panic → Invalid；编译中关机立即返回 Shutdown，代号与设置文件不变）、`a_publication_during_the_compile_refuses_the_candidate`（编译被阻塞期间发布一次，候选必须以 Busy 返回）。

`rtdns check -d <生产快照的原生根> -c config_custom.yaml`：基线与本改动的 stdout/stderr 逐字节相同（`SUMMARY gaps=0 cascades=0 globals=0 definitions=128`，退出码 0）。

## 内存验收（500 qps，生产规则集，单位 MiB）

方法：`tools/5d-2/scenarios.py M0|M1|M3`。被测进程绑一个 CPU，开环 500 qps 加权生产语料，150 s 负载；M1 在 40–90 s 每 10 s 保存一次设置（代理地址来回切换），M3 同时刻交替创建/删除带专用端口的专属组；
35 s 时读 `/proc` 并重置 VmHWM，145 s（负载结束前）与之后空闲 300 s 各读一次。M0 = 同一二进制、同样流量、不做热应用。
判定（用户决定）：6 次热应用后的 RSS ≤ 同一时刻 M0 的 1.2 倍；应用期间 VmHWM ≤ 256 MiB。

### 最终二进制（本记录全部改动；每个应用运行与同一轮的 M0 配对）

| 运行 | 145 s（负载结束前） | 空闲 300 s 后 | 与同时刻 M0 之比（145 s / 空闲） | 应用期间 VmHWM | 空闲时 `[heap]` / 匿名 |
| --- | --- | --- | --- | --- | --- |
| M0（501） | 64.2 | 64.3 | — | — | 36.4 / 16.1 |
| M1 设置 ×6（501） | 69.4 | 69.7 | **1.08 / 1.08** | 149.8 | 28.6 / 29.1 |
| M3 专属组 ×6（501） | 68.4 | 69.2 | **1.07 / 1.08** | 152.1 | 28.6 / 28.7 |
| M0（502） | 64.0 | 64.1 | — | — | 36.0 / 16.1 |
| M1 设置 ×6（502） | 68.2 | 68.7 | **1.07 / 1.07** | 149.8 | 28.1 / 28.5 |
| M3 专属组 ×6（502） | 70.2 | 70.7 | **1.10 / 1.10** | 151.3 | 30.4 / 28.2 |

**通过**：全部 ≤ 1.2（最大 1.10），峰值 ≤ 152.1 MiB（≤ 256），空闲 300 s 内不再增长；全部 75 000 个查询正确，0 丢失、0 内核丢包。

### 各项改动的贡献（同一方法）

| 二进制 | M1 / 同时刻 M0（空闲 300 s 后） | 应用期间 VmHWM | 空闲时 `[heap]` / 匿名（M0 约 36 / 16） |
| --- | --- | --- | --- |
| 基线 `ff885307`（第一段，M1×2、M3×2） | 110.9–111.5 / 64.5 = **1.72–1.73** | 167.0–168.0 | 69.8–70.2 / 28.8 |
| 只有“解除启动代常驻引用” | 87.3 / 64.8 = **1.35** | 142.3 | 58.8 / 15.7 |
| 两项都做（Codex 修复前的二进制，M1、M3 各一次） | 69.2 / 64.5 = 1.07；71.4 / 64.5 = 1.11 | 152.7、153.2 | 28.4–30.5 / 28.4–28.5 |
| 两项都做（最终） | 1.07–1.10 | 149.8–152.1 | 28.1–30.4 / 28.2–29.1 |

- 解除常驻引用：匿名映射回到对照水平（第一次热应用后启动代那一整代不再常驻，峰值也低了一代）；剩下的是主 arena 碎片（`[heap]` 58.8 对约 36）。
- 编译移到一次性线程：候选代与编译临时数据不再和缓存条目交错在主 arena 里，碎片消失；活代的大部分落在编译线程的 arena（计入“匿名”），所以 `[heap]` 反而低于对照、匿名高于对照，合计只多 4.2–6.6 MiB。
  代价是峰值比“只解除引用”高约 10 MiB（编译线程的 arena 与主 arena 不共用空闲块），仍远低于 256 MiB。

## 延迟与正确性（只作参考，不设门槛）

全部场景 0 错误答案，H1 的新代理与新列表在保存后生效、还原后回到原值，H3 的专用端口随保存出现、随删除消失（场景自带检查全部通过，含基线下不通过的 `every_window_healthy`）。

| 场景（500 qps，最终二进制） | 丢失 | 53 端口内核丢包 | 操作窗口 P99（ms） | 操作窗口最大（ms） | 管理 API（s） |
| --- | --- | --- | --- | --- | --- |
| H0 对照（无操作）×1 | 0 | 0 | 11.8 | 15.6 | — |
| H1 设置 ×3（每次 2 个窗口） | 0 | 0 | 13.2–16.8 | 53.0–90.5 | 0.41–0.58 |
| H3 专属组 ×3（每次 2 个窗口） | 0 | 0 | 13.4–17.5 | 54.2–102.7 | 0.44–0.61 |
| 基线 `ff885307`（第一段）H1/H3 ×3 | 0 | 0 | 365–553 | 501–705 | 0.46–0.73 |

编译期间 DNS 照常应答；剩下的数十毫秒来自仍在 DNS 线程上的发布（`domain_output` 统计交接时的同步存盘与读盘）和旧代释放（第一段测量各约 20–30 ms），用户决定本任务不处理（设计 3.2/3.3 留作可选后续）。

## 清理

- 全部 19 次场景运行（含 Codex 修复前二进制的中间运行与“只做 3.1”的运行）与 2 次预言运行的 `cleanup.json` 均为“自有进程已退出、netns 已删除、原有服务未变”；一次中间运行在修复后被主动终止（SIGTERM，清理同样干净），其不完整数据已丢弃。
- 结束时 VM 上没有残留进程与 netns，原有 `mosdns` 服务 PID 未变；VM 任务目录（约 23 GB，含构建产物与私有原始数据）已删除，生产快照原样保留。

## 审查

Codex（`gpt-6.1-sol`/medium，只读）第 1 轮 1 项 P1：专属组保存在等待编译期间若有设置保存发布，候选会用旧视图里的设置配上发布后才读到的 `settings.json` 指纹，通过提交点检查后在内存里撤销刚生效的设置。
已修（`compile_candidate` 的代号守卫与回归测试）；第 2 轮 `FINAL: PASS`，无 finding。
