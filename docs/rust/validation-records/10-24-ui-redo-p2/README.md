# 10-24 UI 重做第二段：规则设置 8 个子项

任务 `10-24-rtdns-ui-redo-p2`，基线 `45f8eb69`；file 仓库基线 `a9e3d54`（只改工作树，见下）。设计与用户决定见任务的 `prd.md` 与 `10-23-rtdns-ui-redo/design.md`（本地，不进仓库）；
旧规则页每个操作的去向见任务的 `research/mapping.md`。契约：[native-feature-flags](../../contracts/native-feature-flags.md)（`requery_on_change`）、
[native-webui-hosting](../../contracts/native-webui-hosting.md)（UI 结构）。

## 改动

| 位置 | 内容 |
| --- | --- |
| `rust/native-host/src/config/flags.rs`、`settings_api.rs` | `feature_flag` 新增可选参数 `requery_on_change`（bool，默认 false，非 bool 为加载错误）。`POST/DELETE /api/v1/flags/{key}` 在值实际改变并发布后，若配置有 `requery` 插件，在**新世代**上启动一次任务；返回里加 `requery_triggered`，`GET` 每项加 `requery_on_change`。值没变、400/404/409/503（含更新试运行）、无 `requery` 插件都不触发且不报错；任务没能启动时改动保留并记一条警告 |
| `rust/native-host/tests/feature_flags.rs`、`feature_flags_trial.rs` | 新增 2 个测试 + 扩展 2 个（声明校验、试运行下状态保持空闲） |
| `webui-log/src/pages/rules/`（8 页） | 运行模式、上游设置、分流规则、请求屏蔽、域名学习、缓存设置、客户端、专属分流组；每页顺序为开关 → 名单/订阅 → 操作；`#/rules/rewrite` 重定向到 `#/rules/routing` |
| `webui-log/src/components/rules/` | `FlagSwitches`（按声明的 `section` 显示、点一下生效、应用中禁用同页其它开关、409/503 处理、已修改/恢复默认）、`ListEditor`（共用名单编辑器）、`SubscriptionTable`（在线订阅与 AdGuard 共用表）、`UpstreamPanel` + `UpstreamEditorModal`（压缩后的上游表与编辑弹窗）、`OverridesPanel`、`DomainGenerationPanel`、`LearningData`（类别与学习明细）、`RequeryPanel`、`CachePanel` + `CacheEntriesModal`、`ClientAliasPanel`、`SpecialGroupDetail`（一个组的全部管理）、`SpecialGroupLinks` |
| `webui-log/src/api/flags.js`、`utils/{ruleList,unsaved,subscription,clientAliases}.js` | 开关分页与结果文案、名单读取/截断保护/搜索、未保存登记、订阅辅助、别名辅助 |
| `router.js`、`main.js`、`App.vue` | 离开有未保存改动的页面/子项/专属组时先确认；关闭标签页由浏览器确认 |
| `QueryManager.vue` | 删除别名弹窗，"客户端别名"按钮改为跳到 `#/rules/clients` |
| 删除 | `ListManager.vue`、`RulesManager.vue`、`UpstreamManager.vue`、`DataManagementManager.vue`、`components/data/*`、`components/rules/` 里 6 个旧子组件、`system/SystemOverridesPanel.vue`、`system/SystemDomainGenerationPanel.vue`、两个占位/提示页；`api/nativeManagement.js` 里只服务旧组件的函数（`validateDiversionCatalogChange`、`supportsEndpoint`、`loadNativeLocalProfiles`、`classifyDiversionSource`、`buildNativeDiversionPayload`）；`NATIVE_REPLACEMENTS_NOTE`；`style.css` 中只服务于已删组件的 209 条规则（postcss 按选择器删除，剩余规则不变） |
| file 仓库 | `config_rust/sub_config/flags.yaml`：`core_mode`、`dns_mode`、`cn_fakeip` 加 `requery_on_change: true`（配置 schema 3 未发布，不升号）；删除 `config_rust/ui/ilog/`；`README-rust.md` 同步（联动参数、ilog 移除）。未动 `config_rust_up.zip`、`config_rust.zip` |
| 文档 | `native-feature-flags.md`、`native-webui-hosting.md`、`contracts/index.md`、`plans/m2-backlog.md`（M2-4 #1 已处理）、`release.md`（v0.3.0 升级须知草稿） |

## 与 PRD 的差异

- 专属分流组的订阅不是"本地 .txt 规则"：标准配置里 `special_route_N` 是 `sd_set_light`（POST/PUT 要求 `files` 与 `url`，后端在 `url` 缺失时返回 400），所以专属组里的订阅与五个内置集合用同一个在线订阅表，旧页面里"本地文本规则"分支（只属于已退役的托管模式）随旧组件删除。
- "域名排行"（`top_domains`）一行与它的清空随旧页面删除：概览的排行来自审计记录，不读 `top_domains`。
- 配置包 `ui/` 目录里的 `modify instruction`（旧 ilog 页的修改说明）未动，只删了 `ilog/`（PRD 只写 `ilog`）。
- requery 的 `webinfo/requeryconfig.json` 是用户运行数据，包里的示例 `flush_rules` 清 `cache_all`、`cache_all_noleak`、`cache_cn`、`cache_google` 与各学习类别（学习类别被保护跳过），不含旧 WebUI 切 DNS 分流模式时额外清的 `cache_google_node`、`cache_cnmihomo`；联动按 PRD 交给 requery 自己的 `flush_rules`，是否补上这两项由用户决定（见汇报）。

## 构建与测试

| 项 | 结果 |
| --- | --- |
| `npm run build`（`webui-log`） | 通过 |
| `node --test webui-log/tests/*.test.mjs` | 76 / 76 通过（P1 结束时 56；新增 `rulesPages` 20 项：开关分页与 409/503/联动文案、名单截断保护与"读取成功前不能保存"、搜索与未保存比较、路由守卫（含同页切换专属组）、整值保存的读取守卫、订阅/别名辅助、各页结构、专属组一处管理、学习页无单条删除；`uiShell` 加 1 项（规则页文案不含内部术语）并改 3 项；旧组件相关的 4 项测试删除/改写） |
| VM `cargo fmt --check` | 通过 |
| VM `cargo clippy --workspace --all-targets -j2 --offline -- -D warnings` | 通过 |
| VM `cargo test --workspace -j2 --offline`（`CARGO_INCREMENTAL=0`、任务专属目录） | 1500 通过、0 失败（基线 1498：新增 2 个，扩展的 2 个计入原数）；最终源码复跑结果相同 |
| 新 `config_rust`（file 工作树）用新程序 `rtdns check` | `SUMMARY gaps=0 cascades=0 globals=0 definitions=118` |

## 真机预览

macOS 上用本次构建的 `rtdns`（debug），配置根为仓库外 `config_rust` 副本：所有监听改到 `127.0.0.1` 的 20000 以上端口（DNS 20053、管理 29099），所有上游指向本机假上游
`udp://127.0.0.1:15399`（A 回 `198.51.100.x`、AAAA 回 `2001:db8::x`），ECS 改为 `2001:db8::8`，requery 定时关闭；名单、订阅地址、别名、学习条目换成示例数据（`*.example.*`、
`192.0.2.0/24` 等）；灌入 400 条 `*.example.*` 查询。前端用 Vite 开发服务器代理到该实例，不访问公网。

| 检查 | 结果 |
| --- | --- |
| 8 个子项逐一可达，开关区按声明的 `section` 与声明顺序显示 | 通过 |
| bool 开关（`cn_fakeip`）：点一下生效、显示"已修改"、提示"正在刷新缓存与预热"，requery 状态随即为运行中；choice 开关（`dns_mode`）：同样提示；"恢复默认"回到 `fakeip` | 通过 |
| 手动名单：`realiplist` 追加一行保存，`/plugins/realiplist/show` 回读一致；提交只含无效行时服务端丢弃，界面提示"部分规则未被服务器接受"并按服务器内容更新 | 通过 |
| 名单截断保护：把 `ddnslist` 文件写成 20,500 行，页面显示"超过 20,000 条"、文本框只读、保存按钮禁用 | 通过 |
| 搜索只过滤显示（文本框隐藏、内容保留）；"还原"丢弃编辑；有未保存编辑时点别的子项弹出确认，取消则留在原页 | 通过 |
| 订阅：`cuscn` 启用开关切换并回读，再切回；编辑弹窗字段 | 通过 |
| 专属分流组：新建（名称、端口 25353、一个专属上游）→ 自动打开该组 → 写入手动名单并回读 → 新增订阅（地址自动识别名称与文件）→ 删除订阅 → 删除组（确认文案列出随组删除的内容）；`rule/special_50.txt`、`srs/special_50.json` 随组删除 | 通过 |
| 上游：编辑弹窗改查询超时并回读，组标题出现"已修改"；表格只有 标识、协议、地址、启用、延迟、操作 | 通过 |
| 域名学习：总开关与分类开关读取；明细按类别筛选、搜索（防抖）、分页；整类清空（确认）后条数归零 | 通过 |
| requery：状态、运行中进度、定时开关与间隔保存并回读（随后关闭） | 通过 |
| 缓存：统计表、条目查看、单个清空（确认）后条目数归零；说明里的跳转指向域名学习 | 通过 |
| 客户端：别名编辑 + 手动加入 + 保存全部并回读；客户端 IP 名单读取 | 通过 |
| 375 px：9 个页面（8 个子项 + 打开一个专属组）`scrollWidth == 375`，无页面级横向滚动（宽表在自身容器内横滑；三选一分段按钮在窄屏换行） | 通过 |

截图（深色、浅色各 8 个子项，1280 宽；运行模式、分流规则、打开一个组的专属分流组各一张 375 px）只放在仓库外的任务目录，未进仓库。

## 审查

Codex CLI（`gpt-6.1-sol`，medium，只读），3 轮：

| 轮 | 结论 | 处理 |
| --- | --- | --- |
| 1 | FAIL：4 个 P1 + 2 个 P2 | P1（1）名单首次读取失败后仍可保存会整体替换名单 → 新增"已成功读取"状态，读取成功前文本框只读、保存禁用；（2）保存期间继续输入会被覆盖 → 保存期间文本框禁用；（3）专属组"保存修改"后整页重挂载丢失名单草稿 → 页面只随所选组重挂载，组表单仅在没有未保存改动时才被刷新数据替换；（4）专属上游编辑会丢掉不支持的字段 → 专属上游同样走只读检查。P2：恢复上游表的按组筛选/排序/隐藏未启用；嵌入产物重新构建。同类顺带修复：别名读取失败不再当空表、覆盖设置与 requery 定时表单在读取成功前不能保存 |
| 2 | FAIL：1 个 P1（保存请求期间的组表单编辑被误标为已保存，随后被服务器旧值覆盖） | 保存后的基线取请求发出时的快照（`submitted`），请求期间的改动仍是未保存 |
| 3 | PASS（无 P0/P1） | — |

没有遗留的 P2（`deferred-p2.md`：无）。

## 最终复跑

最终源码（含上面所有修复与重新构建的内置 UI）在 VM 上复跑：`cargo fmt --check` 通过、`clippy --workspace --all-targets -D warnings` 通过、`cargo test --workspace` 1500 通过 / 0 失败；
前端 `node --test` 76 / 76、`npm run build` 与 `npm run build:external` 通过。内置 UI 用 `npm run build`（Vite 默认时间戳版本号）构建；发布前仍需从干净提交按 `scripts/build-rust-native.sh` 重新盖版本戳。
