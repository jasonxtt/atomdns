# 10-23 UI 重做第一段：框架、概览、日志、系统管理

任务 `10-23-rtdns-ui-redo-p1`，基线 `3028a00a`（M2-1 已合入，配置 schema 3）；file 仓库 `a9e3d54`（未改）。设计与用户决定见任务的 `prd.md` 与
`10-23-rtdns-ui-redo/design.md`（本地，不进仓库）；现有面板/操作的去向见任务的 `research/mapping.md`。
契约：[native-webui-hosting](../../contracts/native-webui-hosting.md)（UI 结构、外观接口删除）、[native-update](../../contracts/native-update.md)（`update_from_url` 删除）。

## 改动

| 位置 | 内容 |
| --- | --- |
| `webui-log/src/App.vue`、`main.js`、`router.js`、`navigation.js`（新） | C 布局：常驻侧栏（规则设置 9 项、系统管理 5 项直接展开），≤760 px 时为两行可横滑标签；`vue-router` hash 路由，刷新保持当前页，未知地址回概览 |
| `webui-log/src/pages/`（新） | 概览、日志、规则设置 9 个子项（挂旧页面或占位）、系统管理 5 个子项 |
| `OverviewPage.vue`（由 `OverviewManager.vue` 改写） | 去掉趋势图与本地历史；7 个数字卡（查询数量、平均响应、缓存命中、客户端数量、域名统计、CPU、内存）+ 覆盖范围；学习状态（`/plugins/route_learner/status`）；上游统计按阈值上色；排行与详情弹窗、筛选跳转日志保留 |
| `utils/overviewMetrics.js`（新） | 阈值常量（一处定义）、上色函数、CPU 单核百分比、进程/缓存指标解析、覆盖范围文字 |
| `utils/theme.js`、`utils/autoRefresh.js`（新） | 浅色/深色/跟随系统（默认跟随系统，`rtdns-theme`）；自动刷新（`rtdns-auto-refresh`） |
| `api/runtimeCapabilities.js` | 能力 ID 去掉 `switches.manage`、`appearance.server`，加 `flags.manage`；删除 Go legacy（发现 404）与旧原生无 `ui_operations` 的回退、switch 标签/世代头；`/plugins/route_learner/{status,entries}` 归 `lists.remembered` |
| `QueryManager.vue` | 实时查询页头加"诊断抓取"入口（页内展开）；审计只用 v2；别名导出名 `rtdns-aliases-*`；先注册筛选事件再加载（修首次挂载时事件早于监听的竞态） |
| `DataManagementManager.vue` | 新增 `mode="learning"`（域名列表统计 + requery）；删除读已不存在的 switch3 的 `loadCoreMode` |
| 删除 | `SystemControlManager.vue`、`OverviewManager.vue`、`dashboard/*`、`useRealtimeMetrics.ts`、`services/dashboard.ts`、`types/dashboard.ts`、`dashboardFormat.ts`、外观 3 个 util、旧开关面板 4 个、`SystemInfoPanel`、`SystemConfigManagePanel`、`SystemReplacementRulesPanel`、`SystemAppearancePanel`（已无引用）；`echarts` 依赖 |
| `style.css` | 新浅/深两套色板（扁平、无毛玻璃与背景图）、侧栏与数字卡样式；删除只服务于已删外观/旧外壳的规则（postcss 按选择器删除，diff 只有删除行） |
| 改名 | 可见 MosDNS → rtdns；localStorage 键 `rtdns-` 前缀（旧键不读不迁移）；`package.json` `name` 为 `rtdns-webui`；`index.html` 标题；`vite.config.js` 代理默认 `http://127.0.0.1` |
| `rust/native-host/src/appearance.rs`、`appearance/tests.rs` | 删除；`api.rs` 去掉外观路由、上传槽、120 s 上传期限、20 MiB 体积特例与只服务于 multipart 的 `Content-Type` 解析；能力 `appearance.server` 删除 |
| `rust/native-host/src/update/api.rs` | 删除 `POST /api/v1/config/update_from_url`（旧地址 404）；`ConfigRequest` 去掉 `url` |
| 测试 | 前端：`uiShell.test.mjs`（新，路由/主题/阈值/CPU/覆盖范围/键名/CSS）；其余 5 个文件按新结构改写。Rust：`f1_system.rs`（去掉 2 个上传测试，重启后保持的设置改用审计上限，断言外观接口 404）、`f3a_update.rs`（去掉 2 个 `update_from_url` 测试，导出测试断言其 404）、`slice6_management_http.rs`（能力 30 项）、`audit_retention_tests.rs` |
| 文档 | 契约 2 份 + `contracts/index.md`、`release.md`（v0.3.0 升级须知草稿）、`m2-backlog.md`（M2-4 #8 文案已处理）、`feature-coverage.md`（C07/C12 注记）、`docs/ai/project-context.md` |

## 删除项清单

- 前端操作：实时趋势图、"当前请求数/当前处理时间"、最近 5 条查询预览、应用远程配置、外观（字体/按钮颜色、背景图上传/历史/URL/纯色、透明度、毛玻璃、外观重置）、旧开关页（功能开关、已配置开关、核心模式、DNS 分流模式）、"切换 v3"按钮、覆盖保存后的重启提示（Go 分支）、Go 指标/进程提示行。
- 后端：`/api/v1/appearance/*`（全部）、`appearance.server`、`POST /api/v1/config/update_from_url`。
- 部署机上已有的 `webinfo/appearance_*.json`、`panel_background*` 新程序不读不删（升级须知写明可手动删除）。

## 构建与测试

| 项 | 结果 |
| --- | --- |
| `npm ci && npm run build`（经 `scripts/build-rust-native.sh`，macOS） | 通过；`app.js` 323.8 kB（gzip 104.7 kB），单文件无 chunk |
| `npm run build:external` | 通过（包与 zip 在被忽略的 `webui-log/dist-external/`、`release/`） |
| `node --test webui-log/tests/*.test.mjs` | 56 / 56 通过（基线 47 / 47；新增 `uiShell` 12 项，删改过时项） |
| VM `cargo fmt --check` | 通过 |
| VM `cargo clippy --workspace --all-targets -j2 -- -D warnings`（离线，`CARGO_INCREMENTAL=0`） | 通过 |
| VM `cargo test --workspace -j2`（离线） | 1498 通过、0 失败（98 个测试二进制）；基线 1516（删除外观单元测试 14、上传 2、`update_from_url` 2） |

## 本机真机预览

macOS 上用 `build-rust-native.sh` 的 rtdns，配置根为仓库外 `config_rust` 副本（file `a9e3d54`）：所有监听改到 `127.0.0.1` 的非特权端口（DNS 15353、管理 19099），
所有上游指向本机假上游 `udp://127.0.0.1:15399`（A 回 `198.51.100.x`、AAAA 回 `2001:db8::x`），ECS 改为 `2001:db8::8`，requery 定时关闭；灌入 400 条 `*.example.{com,net,org}` 查询。
macOS 没有进程指标（`metrics.process` 不支持，卡片显示原因），且在线更新只支持 linux/amd64，所以同一份配置又在 VM 上用 Linux release 构建跑了一遍
（`MOSDNS_UPDATE_API_URL` 指向本机死地址，不访问公网），截图取自 VM（经 SSH 隧道）。

| 检查 | 结果 |
| --- | --- |
| 16 个侧栏项逐一可达、标题正确、无报错提示、控制台无错误 | 通过 |
| 刷新保持（`#/rules/learning` 刷新后仍在原页）；未知地址（`#/bogus`）回概览；`#/rules`、`#/system` 跳到第一个子项 | 通过 |
| 深色 / 浅色 / 跟随系统（系统深浅切换时跟随，固定模式不跟随）；保存在 `rtdns-theme`，刷新后保持 | 通过 |
| 375 px：16 个页面 `scrollWidth == 375`，无页面级横向滚动；侧栏变为顶部两行标签 | 通过（宽表在自身容器内横滑） |
| 概览：数字、覆盖范围、阈值上色、CPU（首轮后 3 s 补一次采样）、内存（Linux）、学习状态、排行详情、最慢查询详情 →"筛选"跳到日志并带入搜索词 | 通过 |
| 日志：诊断抓取入口展开/收起 | 通过 |
| 挂入旧页面读写：上游设置保存 ECS 覆盖并回读、"恢复默认"；专属分流组新建"测试组"；分流规则里给该组手动名单写入一行并经 `/plugins/special_manual_50/show` 回读（落盘到 `rule/special_50.txt`）；域名学习开关读取、缓存列表读取 | 通过 |
| 配置与备份：导出 200 zip、非本机目录 400；`update_from_url` 与 `/api/v1/appearance/*` 均 404 | 通过 |
| 版本更新：系统信息（版本、启动时间、运行时长、CPU 时间、RSS、FD、线程）、检查更新失败时的说明、配置包状态（schema、包 ID、更新事务） | 通过（Linux） |

截图（概览、日志含诊断入口、系统管理 5 项、分流规则、运行模式占位；深浅各一套；概览与日志各一张 375 px）只放在仓库外，未进仓库。

## 与 PRD 的差异

- 客户端数量与域名统计取自排行接口（上限 500 项），达到上限显示"500+"。
- 系统信息放在"版本更新"页顶部；后端不提供配置根路径，所以没有显示配置根，备份仍需手填配置目录。
- 下载文件名由前端固定为 `rtdns_backup.zip`；后端 `Content-Disposition`（`mosdns_backup_<unix>.zip`）未改。
- 缓存命中是所有命名缓存计数器之和；整代热应用后计数器重新开始，此时显示 "--" 直到有新的缓存查询。

## 审查

Codex CLI（gpt-6.1-sol，medium，只读）第 1 轮：`FINAL: PASS`，无 P0/P1；1 个 P2（首轮 CPU 采样时间记录过晚，排行慢时百分比偏高），已直接修复（采样时间取 `/metrics` 返回时刻），修复后重新构建与前端测试通过。

## 返工 1（2026-10-23，用户看截图后的决定，见任务 `rework-1.md`）

| 位置 | 内容 |
| --- | --- |
| 概览 | 删除"时间段统计"表（`/api/v2/audit/stats/windows` 只用来取覆盖范围）；上游统计与学习状态同一行（约 3:1，窄屏上下排列、等高）；上游统计删除"出错数"列，列宽与行高收紧，地址列截断、悬停看全；数字卡 4+3 两行，说明文字不截断 |
| 日志 | 诊断抓取改为三级页面 `#/logs/diagnostic`（`LogsDiagnosticPage.vue`）：实时查询页只有页头小按钮，诊断页顶部"返回实时查询"；两页互不显示对方内容，刷新保持，侧栏"日志"在两页都高亮 |
| 系统管理 | 合成一页 `#/system`（`SystemPage.vue` + 五个分段组件 `System{Update,Backup,Audit,Ui,Service}Section.vue`）；统一设置行组件 `components/settings/SettingRow.vue`、`SettingSection.vue`（名称 / 说明 / 当前值与操作）；旧 `#/system/<子项>` 重定向到 `#/system`；1280×800 一屏看完（页面底部 750 px，无滚动） |
| 文案 | 审计说明改为"最多保存多少条查询详情……修改上限会清空已保存的详细日志"等，去掉 retained ring、v2 统计、时间窗、热数据等内部词 |
| 排版 | 设置行里按钮不换行、输入框与按钮同高（30 px）、各段同宽；概览同一行面板等高 |
| 其它 | `QueryManager.vue` 别名占位示例改为 `192.0.2.10`；删除只服务于旧系统子页的 `SystemUpdatePanel`、`SystemAuditPanel`、`SystemAuditCapacityPanel`、`SystemAutoRefreshPanel`、`SystemWebuiPortPanel` 及其样式 |

验证：`node --test` 59 / 59（`uiShell` 新增诊断页、系统分段、文案 3 项，路由测试改为新地址）；`scripts/build-rust-native.sh` 构建通过；后端未改，Rust 测试沿用上文结果。
预览仍在 VM（Linux release 构建、同一份测试配置、更新地址指向本机死地址）+ 本机 Vite（代理到 VM）上进行：13 个路由在 375 px 下无页面级横向滚动；
诊断页进入、刷新保持、返回；`#/system/audit` 重定向；系统页审计停止/开始、自动刷新开关的写入与回读。截图在任务目录 `screenshots-r2/`（不进 git）。

Codex CLI 第 2 轮（只审返工与第 1 轮 finding）：`FINAL: PASS`；第 1 轮 P2 确认已修复，返工无新 finding。
