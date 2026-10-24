# 10-24 v0.4.0：WebUI 拆到 `ui/main`、开关与上游显示修正、侧栏折叠

任务 `10-24-rtdns-v0.4.0-ui-split`，基线 `2fa09319`（v0.3.0）；file 仓库基线 `c84bcc8`（只改工作树）。设计与用户决定见任务的 `prd.md`、`research/design.md`、`phase2-go.md`（本地，不进仓库）。
契约：[native-webui-hosting](../../contracts/native-webui-hosting.md)、[native-update](../../contracts/native-update.md)、[native-settings](../../contracts/native-settings.md)；操作说明：[external-ui](../../external-ui.md)、[headless](../../headless.md)、[release](../../release.md)（v0.4.0 升级须知草稿）。
本任务不改版本号与发布元数据（`scripts/rust-release-metadata.json` 仍是 schema 3，发版任务更新它之前 `generate-rust-update-manifest.sh` 会拒绝生成清单——预期的护栏）。

## 改动

| 位置 | 内容 |
| --- | --- |
| `rust/native-host/build.rs`、`Cargo.toml`、`src/static_ui.rs`；删除 `rust/native-host/ui/` | 程序不再内嵌 WebUI（`build.rs` 只处理版本号，去掉 `ring` 构建依赖）；`static_ui.rs` 只剩共用的解码/MIME/重定向与"没有安装 WebUI"页（404，`text/html`，附 `ui/main` 被拒的原因并转义）；不再解析 `If-None-Match` |
| `src/external_ui.rs`、`src/api.rs` | `ui/main` 由 `/` 提供（不在 `/main/`）；路由：已挂载的 `/<名字>/` → `/api*`、`/plugins*`、`/metrics` 一律交给接口 → 其余 `GET/HEAD` 取 `ui/main` 的文件 → 其它方法交给接口；保留名加 `chunks`；健康文档 `external_ui` 每项加 `path` |
| `src/config/flags.rs`、`config/settings.rs`、`settings_api.rs`、`api.rs` | `aliapi` 可选参数 `show_when`（条件写法同 `flag` 匹配器，加载时校验）；`GET /api/v1/upstream/groups`（`active`、`show_when`、`inactive_reason`），能力 `endpoints.upstream.groups_get`；上游列表非空但没有启用项 → 中文 400（挂在开关上的组提示关闭该开关），不再把 `aliapi` 编译错误抛给界面；`[]` 仍是"恢复默认" |
| `src/update/{mod,api,transaction,guard,archive}.rs`、新 `update/webui.rs` | `WEBUI_ASSET = "rtdns-webui.tar.gz"`（清单 `artifacts` 的一个键）；`ui/main` 是真实目录时下载、校验 sha256、暂存前完整解压检查（只收普通文件与目录、路径与数量/大小上限、`index.html` 与 `ui-manifest.json`）；同文件系统检查；守护进程在配置包之后、程序之前用两次 rename 换入，回滚可重复；候选健康要求 `main` 已挂载；清单缺界面资产时拒绝整个更新；`ui/main` 是符号链接时不替换并在 `notes` 说明；事务文件新增 4 个可选字段；`REQUIRED_CONFIG_SCHEMA = 4` / `rust-config-schema-4` |
| `webui-log/` | 一份 `vite.config.js`（`base: './'`，输出 `dist/`，删 `vite.external.config.js`）；`npm run build` / `npm run build:package`（`scripts/build-webui.mjs`，取代 `build-external-ui.mjs` 与 zip）；侧栏"规则设置"默认折叠、点击只展开/收起、进入规则子页自动展开（手机同样）；上游设置隐藏未生效的组（"显示未生效的组（N）"）、关闭/删除最后一个启用的上游时先拦下并说明、被拒后开关复位；概览上游统计不显示未生效的组；系统页"界面版本"，`main` 不再列为外置界面；DDNS 名单提示指向运行模式 |
| `scripts/` | `build-rust-native.sh` 只构建程序；`native-build-manifest.mjs` 不再收集 UI；`build-rust-release.sh` 产出 `rtdns-webui.tar.gz` 并核对 `ui_version == tag`；`generate-rust-update-manifest.sh` 要求界面包并比对第三个资产名 |
| file 仓库（工作树） | `flags.yaml`：`ddns_foreign` 的 `section` → `mode`；`forward_1.yaml`：`cnfake` `show_when: flag cn_fakeip`、`nocnfake` `show_when: flag dns_mode=fakeip`，两处 requery 分支按 `dns_mode` 走（D3）；`build_config_rust_up.sh` 两行升到 schema 4；新 `build_config_rust_zip.sh <rtdns-webui.tar.gz>`；`README-rust.md`。未动 `config_rust.zip` 与未跟踪的 `config_rust_up.zip` |
| 文档 | 上列契约与操作说明、`contracts/index.md`、`AGENTS.md`、`README.md`、`docs/ai/project-context.md`、`repository-layout.md` |

## D3：RealIP 模式下刷新把 FakeIP 写进共用缓存（实测）

VM 网络命名空间（只有回环），完整 `config_rust`，上游全是本地假 DNS：domestic → 无 A；foreign → `203.0.113.10`；foreignecs → `203.0.113.20`；sing-box（`nocnfake`，6666）→ `198.51.100.77`（FakeIP 替身）；cnfake → `198.51.100.88`。
测试域名 `d3test.example.com` 加入白名单。先查刷新端口 3066（相当于一次 requery），再查客户端端口 53。

| 运行 | 程序 / 配置 | dns_mode | 3066 应答 | 之后 53 应答 | sing-box 被问 |
|---|---|---|---|---|---|
| 对照 | v0.3.0 / 原配置，不刷新 | realip | — | 空 | 0 |
| 修复前 | v0.3.0 / 原配置 | realip | 198.51.100.77 | **198.51.100.77** | 1 |
| 修复后 | v0.3.0 / 新配置（去掉它不认识的 `show_when`） | realip | 203.0.113.10 | 203.0.113.10 | 0 |
| 修复后 | 同上 | fakeip | 198.51.100.77 | 198.51.100.77 | 1（行为不变） |
| 修复后 | v0.4.91 测试构建 / 完整新配置 | realip | 203.0.113.10 | 203.0.113.10 | 0 |
| 修复后 | 同上 | fakeip | 198.51.100.77 | 198.51.100.77 | 1 |

## cnfake 的两个现象（复现于第一段）

本机 v0.3.0 源码的 debug 构建 + 只含 `cnfake` 组的测试配置：停用唯一上游 → `400 $.plugins[1].args.upstreams: upstreams must not be empty`；删除唯一上游（界面发 `[]`）→ `200 saved`，但 `[]` 的含义是"恢复默认"，刷新后 YAML 里那条又出现。
修复后：停用 → `400 上游组「cnfake」至少要保留一个启用的上游。这个组只在「国内域名 FakeIP」开启时使用；不想用它，关闭该开关即可。`，什么都不保存；界面在发请求前就拦下删除与停用（集成测试 `v040_upstream_groups.rs`、前端测试、预览截图）。

## 构建与测试

| 项 | 结果 |
| --- | --- |
| `npm run build` / `npm run build:package`（`webui-log`） | 通过；包内 `index.html`、`app.js`、`app.css`、`ui-manifest.json`，`ustar`、属主 0 |
| `node --test webui-log/tests/*.test.mjs` | 80 / 80（基线 76：新增侧栏折叠、`show_when` 分组映射、最后一个上游的拦截与开关复位、上游页/概览隐藏；改写构建与"程序不内嵌 UI"相关 3 项） |
| VM `cargo fmt --check` | 通过 |
| VM `cargo clippy --workspace --all-targets -j2 -- -D warnings` | 通过 |
| VM `cargo test --workspace -j2 --no-fail-fast`（`CARGO_INCREMENTAL=0`、任务专属目录） | 最终源码 1522 通过、0 失败（`#[test]` 1494 → 1513：新增 tar 树解压 2、WebUI 事务 8、清单兼容 1、"未安装"页 1、根挂载 2、HTTP 集成 2、`show_when`/分组/拒绝 2、f3a WebUI 场景 3；删去内嵌 UI 的 2 项；f3a 原有 3 项扩展为带 `ui/main`） |
| 新 `config_rust`（file 工作树）用新程序 `rtdns check` | `SUMMARY gaps=0 cascades=0 globals=0 definitions=118` |
| `generate-rust-update-manifest.sh` 冒烟（临时 schema 4 元数据） | 两个资产都登记；缺 `rtdns-webui.tar.gz` 时拒绝；用仓库里未更新的元数据时按预期拒绝 |
| `build_config_rust_zip.sh`（输出到仓库外） | zip 含 `ui/main/` 的 4 个文件，其余与 `config_rust/` 相同 |

## 更新事务演练（VM，网络命名空间，本地假 Release 服务器与假 DNS，完整 `config_rust`）

v0.4.90 / v0.4.91 是同一源码的测试构建（仅版本号不同，不发布）；v0.3.0 为基线源码构建；配置包是工作树 `config_rust` 生成的 schema 4 包。

| 场景 | 结果 |
| --- | --- |
| 有 `ui/main`：v0.4.90 → v0.4.91 | PASS：`notes` 为"二进制、界面与所需配置已下载并校验"；之后版本 v0.4.91、`ui/main` 版本 v0.4.91、`/` 200；界面资产下载 1 次 |
| 无 `ui/main`：v0.4.90 → v0.4.91 | PASS：升级成功；`ui/main` 未创建、界面资产下载 0 次、`/` 仍 404（纯后端） |
| 候选失败回滚（新界面要求接口级别 9，新程序拒绝挂载） | PASS：15 秒内未通过健康检查 → `rolled_back`；程序 v0.4.90、`ui/main` v0.4.90、`/` 200 |
| 守护进程被 `kill -9`（候选试运行中） | PASS：被杀时盘上是 v0.4.91 程序 + v0.4.91 界面；再启动时"检测到未完成的更新，已在启动前自动回滚"，程序 v0.4.90、`ui/main` v0.4.90 |
| v0.3.0 → v0.4.91，先解压界面再在线升级 | PASS：升级前 v0.3.0 把 `ui/main` 挂在 `/main/`；升级后 `/` 即 `ui/main`（v0.4.91）；v0.3.0 的更新器不下载界面资产（0 次），清单多出的 `artifacts` 键不影响它 |
| v0.3.0 → v0.4.91，先在线升级再解压 | PASS：升级后 `/` 为"没有安装 WebUI"（404）；解压到 `ui/main` 并重启后 `/` 200、界面版本 v0.4.91 |

单元/集成测试另覆盖：缺界面资产或界面资产 sha 不符时整次更新被拒且不留事务、两次 rename 之间断电由启动恢复复原、回滚重复执行无副作用、`ui/main` 为符号链接/文件时不替换、旧格式事务文件可读。

## 预览

VM 网络命名空间里运行 v0.4.91 测试构建 + 完整新 `config_rust`（上游指向本地假 DNS、名单换成示例域名、ECS `2001:db8::8`、更新地址指向本机死端口），`ui/main` 与 `ui/aa` 为同一界面包；经 socat 与 SSH 隧道在本机用无头 Chrome 截图（22 张，只放任务目录 `screenshots/`，不进仓库）。

| 检查 | 结果 |
| --- | --- |
| 侧栏默认收起；点"规则设置"展开/收起且不跳转；进入规则子页自动展开并高亮（深、浅） | 通过 |
| 手机 375：顶栏"规则设置"切换第二行；规则子页默认展开；上游设置页 `scrollWidth == 375` | 通过 |
| 运行模式页有"DDNS 优先国外解析"；上游设置页没有开关区；DDNS 名单提示指向运行模式 | 通过 |
| `cn_fakeip` 关：上游设置与概览都不显示"国内 FakeIP 上游"；"显示未生效的组（1）"后出现并标"当前未生效" | 通过 |
| 停用 cnfake 唯一上游：提示原因与"关闭该开关即可"，开关保持开启，未发请求 | 通过（首轮截图发现开关显示为关，已修正为按重读数据复位后重拍） |
| `cn_fakeip` 开 → cnfake 出现；`dns_mode=realip` → 国外 FakeIP 组隐藏（上游设置与概览）；恢复默认后复原 | 通过 |
| `/` 由 `ui/main` 提供；`/aa/` 外置界面可用；系统页"界面版本 v0.4.91"、外置界面链接 `/aa/` | 通过 |
| 移走 `ui/main` 后重启：`/` 为"没有安装 WebUI"（404），`/api/v1/system/health` 照常 | 通过 |

## 审查

Codex CLI（`gpt-6.1-sol`，medium，只读），2 轮：

| 轮 | 结论 | 处理 |
| --- | --- | --- |
| 1 | FAIL：5 个 P1 + 1 个 P2 | （1）事务文件里的 WebUI 目录可被篡改成 `update/` 或事务目录本身 → 三个字段必须等于本事务的固定路径；（2）安装/回滚时 `ui/` 可能已被换成符号链接 → 移动 `ui/main` 前要求 `ui/` 仍是普通目录，安装时复查同文件系统；（3）读不到旧界面备份被当成"没有备份"，回滚误报成功 → 只有 `NotFound` 算不存在，其余错误让回滚失败并保留状态；（4）解压出的子目录未 fsync → 解压结束自底向上同步全部目录；（5）`ui/main/main/` 会让 `/main/` 返回 200 → `/main` 及其下一律交给接口（404）。P2：生效组的前端拒绝提示缺开关指引 → `/api/v1/upstream/groups` 对有条件的组总给 `used_when`。均补了测试 |
| 2 | PASS（6 条均已修好，无新 P0） | — |

没有遗留的 P2。

## 最终复跑

最终源码在 VM 上：`cargo fmt --check` 通过、`clippy --workspace --all-targets -D warnings` 通过、`cargo test --workspace --no-fail-fast` 1522 通过 / 0 失败；前端 `node --test` 80 / 80、`npm run build` / `npm run build:package` 通过。
演练与预览用的是第 1 轮审查前的构建；审查后的修复只涉及事务路径校验、符号链接/读取错误处理、目录 fsync、`/main` 路由与 `used_when` 字段，由上面的测试覆盖，未重跑演练。
