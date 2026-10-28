# 发版与手动替换（rtdns）

本文是操作说明，不会自动执行任何发布。契约见 [native-update.md](contracts/native-update.md)；
示例路径（`/usr/local/bin/rtdns`、`/cus/rtdns`）按实际部署替换，文中不含任何生产地址。

## 命名

| 项 | 值 |
| --- | --- |
| 仓库 | `jasonxtt/rtdns` |
| tag / Release 名 | `vX.Y.Z`（只认这一种写法；`rust-v*`、`lite-v*`、`vX.Y`、`vX.Y.Z-rc1` 都会被更新器忽略） |
| 程序 | `rtdns`（`rust/native-host` 的 cargo bin；crate 名仍是 `mosdns-native-host`） |
| 发布包 | `rtdns-linux-amd64.tar.gz`（包内只有一个文件 `rtdns`） |
| 界面包 | `rtdns-webui.tar.gz`（v0.4.0 起；顶层即 `<配置根>/ui/main/` 的文件，`ui-manifest.json` 的 `ui_version` 等于 tag） |
| 清单 | `rtdns-update-manifest.json`（`format 1`、`channel "rust"`、`version` 等于 tag、各发布包的 sha256） |
| 程序自报版本 | `rtdns version` 与 `GET /api/v1/system/health` 的 `version` 都等于 tag |

发布版本的 `native-host` crate 版本（`rust/native-host/Cargo.toml`）应与 tag 一致；非发布构建自报 `v<crate 版本>`。

## 发版步骤

1. 确认 `main` 上 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
   `cargo test --workspace --no-fail-fast` 全绿，`rust/native-host/Cargo.toml` 的版本等于要发的版本。
2. 在 Linux x86_64 机器上构建（需要 Node 与能联网的 `npm ci`；不要同时跑前端与 Rust 构建）：

   ```bash
   scripts/build-rust-release.sh v0.2.0 release/rust
   ```

   产物：`release/rust/rtdns-linux-amd64.tar.gz`、`release/rust/rtdns-webui.tar.gz`、`release/rust/rtdns-update-manifest.json`。
   脚本会拒绝非 `vX.Y.Z` 的 tag，要求构建出的程序自报版本、界面包的 `ui_version` 都等于 tag，并核对 `scripts/rust-release-metadata.json`
   与 `update/mod.rs` 里的配置包标识和三个资产名（清单的 `artifacts` 必须同时有程序包与界面包）。脚本只构建，不发布。
3. 核对：

   ```bash
   cd release/rust
   sha256sum rtdns-linux-amd64.tar.gz rtdns-webui.tar.gz
   jq '.version, .artifacts' rtdns-update-manifest.json
   tar -tzf rtdns-webui.tar.gz | head
   mkdir -p /tmp/rtdns-verify && tar -xzf rtdns-linux-amd64.tar.gz -C /tmp/rtdns-verify
   /tmp/rtdns-verify/rtdns version
   ```

   sha256 必须与清单一致，`version` 必须等于 tag。
4. 在 GitHub 建 Release（tag 与 Release 同名，不是 draft、不是 prerelease），把上面三个文件作为资产上传，例如：

   ```bash
   gh release create v0.2.0 release/rust/rtdns-linux-amd64.tar.gz release/rust/rtdns-webui.tar.gz release/rust/rtdns-update-manifest.json \
     --repo jasonxtt/rtdns --title v0.2.0 --notes "说明"
   ```

   这是对外发布动作，需要先得到负责人确认。
5. 仅程序改动的发布到第 4 步为止。若本次发布需要新的配置包（结构性改动，`required_config_schema` 与 `config_package_id` 同时提高），
   配置包 `config_rust_up.zip` 作为**同一个 Release 的资产**，按下面的顺序发版（配置包标识 `channel "rust"`、`rust-config-schema-N`、`config_rust_up.zip` 不随改名变化）。

## 带配置包的发版顺序

1. **出包**：在 file 仓库的 `mosdns/config/` 运行 `build_config_rust_up.sh`（`SCHEMA`/`PACKAGE_ID` 必须等于 `update/mod.rs` 的 `REQUIRED_CONFIG_SCHEMA`/`REQUIRED_CONFIG_PACKAGE_ID`），
   得到 `config_rust_up.zip`；包只作为 Release 资产，不进 file 仓库。把它放在仓库外，**发布用的就是这一份文件**（重新出包会因 zip 时间戳不同而得到不同的 sha256）。
   核对包内 `manifest.json` 的 `config_schema`、`package_id`，以及托管文件列表（含新增文件、不含 `sub_config/special_groups.yaml`）。
2. **填元数据**：`sha256sum config_rust_up.zip`，把 `scripts/rust-release-metadata.json` 的 `config.url`
   （`https://github.com/jasonxtt/rtdns/releases/download/vX.Y.Z/config_rust_up.zip`）与 `config.sha256` 填上，`required_config_schema`/`config_package_id` 同步提高。
3. **提交**：版本号（`rust/native-host/Cargo.toml`、`rust/Cargo.lock`）与元数据一起提交。
4. **构建程序与清单**：Release 还没建，`config.url` 此时下不到包，所以用本地文件校验：

   ```bash
   RUST_CONFIG_PACKAGE=/path/to/config_rust_up.zip scripts/build-rust-release.sh vX.Y.Z release/rust
   ```

   脚本核对该文件的 sha256 与包内 manifest 是否等于元数据，通过后清单带 `config{url,sha256}`。
5. **建 Release**，一次上传四个资产：程序包、界面包、清单、`config_rust_up.zip`（文件名必须与 `config.url` 末段一致）。
6. **全新安装包**：用同一份 `rtdns-webui.tar.gz` 在 file 仓库运行 `mosdns/config/build_config_rust_zip.sh <rtdns-webui.tar.gz>` 重新生成 `config_rust.zip`（带 `ui/main/`）并提交 file 仓库。
   Release 建好后，用 `curl -fsSL <config.url> | sha256sum` 再核对一次与元数据一致。更新器只在 `applied_config_schema < required_config_schema` 时下载配置包。

## v0.4.1 升级须知（2026-10-28；内存优化，仅程序与界面）

- 配置 schema 仍为 4，没有新配置包：在线更新只换程序与 `ui/main`（已安装界面时），不动配置文件与用户数据。
- 常驻内存下降：域名索引池化与审计日志去重，受控测试稳态 RSS −23%（不做操作时约 68 → 52 MiB），热应用峰值约 −17%，日志读/清空/改容量峰值约 −30%；每个应答的 CPU 约 +5.6%。证据见 [10-25-memory-optimization](validation-records/10-25-memory-optimization/README.md)。
- **行为变化**：由 WebUI/接口保存过的上游组（`webinfo/settings.json` 的 `upstreams.<组>`），现在同时向组内**所有启用的上游**并发查询（最多 20 个，与 Go 版一致）；只用 YAML 的组保持自己的 `concurrent`。
  专属分流组同样按启用数并发。想恢复 YAML 的列表与并发数，在上游设置里对该组"恢复默认"。保存与 YAML 相同的列表也会保留为"已管理"。
- 概览"域名统计"改为按完整保留日志计数（以前受排行截断影响会偏小）；DDNS 名单移到"分流规则"页。

## v0.4.0 升级须知（配置 schema 4，2026-10-24；WebUI 拆出程序）

### WebUI 从程序里拆出来

- 程序**不再内嵌 WebUI**。WebUI 是 `<配置根>/ui/main/`，由 `/` 提供（不需要路径前缀）；其它 `ui/<名字>/` 照旧在 `/<名字>/`。没有 `ui/main/` 时 `/` 只显示"没有安装 WebUI"，接口照常（纯后端）。
- 每个 Release 多一个资产 `rtdns-webui.tar.gz`。从 v0.4.0 起，在线更新**只在 `ui/main/` 已存在时**把它与程序、配置包放进同一个事务替换，失败一起回滚；没有 `ui/main/` 的机器保持纯后端。
- 全新安装的 `config_rust.zip` 自带 `ui/main/`。

### 从 v0.3.0 升级（一次性手动步骤）

v0.3.0 的更新器不认识界面资产：它会照常升级程序与配置包（schema 3 → 4），但不会放 `ui/main/`，而 v0.4.0 已不内嵌界面。**先放界面、再在线升级**：

1. 从 v0.4.0 的 Release 下载 `rtdns-webui.tar.gz`，核对 sha256 与同一 Release 的 `rtdns-update-manifest.json` 里 `artifacts["rtdns-webui.tar.gz"].sha256` 一致：

   ```bash
   sha256sum rtdns-webui.tar.gz
   jq -r '.artifacts["rtdns-webui.tar.gz"].sha256' rtdns-update-manifest.json
   ```

2. 解压到配置根（`rtdns start -d` 的目录）下的 `ui/main/`（v0.3.0 会把它当作外置界面挂在 `/main/`，不影响使用）：

   ```bash
   mkdir -p /cus/rtdns/ui/main && tar -xzf rtdns-webui.tar.gz -C /cus/rtdns/ui/main
   ```

3. 在 v0.3.0 的界面里照常"在线更新"到 v0.4.0（自动切换）。
4. 打开 `http://<主机>:<API 端口>/`，系统管理页"界面版本"应为 v0.4.0。

已经先升级了的：`/` 会显示"没有安装 WebUI"，做第 1–2 步后重启一次（`systemctl restart rtdns` 或 `curl -X POST http://127.0.0.1:<API 端口>/api/v1/system/restart`）。
回退到 v0.3.0 时 `ui/main/` 留着无害（v0.3.0 用内嵌界面，把它挂在 `/main/`）。

### 配置（schema 4）

- `ddns_foreign`（DDNS 优先国外解析）移到"运行模式"。
- 上游组可声明 `show_when`：`cnfake` 只在"国内域名 FakeIP"开启时、`nocnfake` 只在"FakeIP 分流"时显示在上游设置与概览（可点"显示未生效的组"查看）；3099 端口（clashmi）不看 `cn_fakeip`，始终使用 `cnfake`。
- 一个上游组至少要保留一个启用的上游：停用或删除最后一个会被明确拒绝（以前停用报看不懂的 400，删除则显示成功、刷新后又回来）。
- 修正：RealIP 分流下，刷新（requery）不再向 sing-box 要 FakeIP 并写进客户端共用的缓存。
- 侧栏"规则设置"的 8 个子项默认收起，点"规则设置"展开/收起。

## v0.3.0 升级须知（配置 schema 3，2026-10-24；功能开关 + UI 重做）

经在线更新升级时，程序与配置包（`rust-config-schema-3`）一起由 update-guard 替换，失败则程序、配置文件、包状态一起回滚。
手动替换程序时必须同时更新配置包：新程序只认 schema 3，旧配置里的 `switchN` 插件或匹配写法在新程序里加载失败。

### 配置与开关

- 包把 `switch1`–`switch17` 换成 `sub_config/flags.yaml` 里的 13 个具名开关（`feature_flag`，见 [native-feature-flags](contracts/native-feature-flags.md)），
  删除 `sub_config/switch.yaml`；`create_if_missing` 为空，`rule/switchN.txt` 是用户数据，包既不创建也不删除。
- 开关值改存 `webinfo/settings.json` 的 `flags` 段（只存与 YAML 默认不同的值）。**首次正常启动**时从 `rule/switchN.txt` 迁移一次；
  在线更新的试运行期间不写盘（`settings.json` 字节不变，所以自动回滚是安全的），由 update-guard 起的最终程序写入。旧的 `rule/switchN.txt` 保留但不再读取。
- 开关可在 WebUI 规则设置各子项顶部切换，也可用 `GET/POST/DELETE /api/v1/flags[/{名字}]`。
  `core_mode`、`dns_mode`、`cn_fakeip` 切换后由程序自己触发一次 requery（清缓存再预热）；自己的配置要同样的联动，在开关的 `args` 里写 `requery_on_change: true`。

### 删除的接口（依赖它们的外置界面会失效）

- `/plugins/switchN/show`、`/plugins/switchN/post`（改用 `/api/v1/flags`）；
- `/api/v1/appearance/*` 与能力 `appearance.server`；
- `POST /api/v1/config/update_from_url`（"应用远程配置"；旧地址返回 404，配置包只随程序在线更新下发）。

调用这些接口的外置界面（旧的 `ilog`、第三方控制台、自写脚本）升级后对应功能会失效，需要改用上面的新接口或停用。"备份配置"保留，下载文件名 `rtdns_backup.zip`。

### 内置 WebUI（v0.3.0 时）

- 侧栏布局：概览、日志、规则设置（8 个子项：运行模式、上游设置、分流规则、请求屏蔽、域名学习、缓存设置、客户端、专属分流组）、系统管理（一页），
  地址形如 `#/overview`、`#/rules/upstream`，刷新保持当前页；`#/rules/rewrite` 跳到 `#/rules/routing`。
- 外观只保留浅色 / 深色 / 跟随系统（存在浏览器里）；背景图、毛玻璃、透明度、文字与按钮颜色已删除。外置 UI 要背景图请放进自己的静态包。
- **浏览器里的旧设置不迁移**（主题、自动刷新、配置目录等 `mosdns*` 键），首次打开新界面按默认值（跟随系统、自动刷新关闭），需要时重新设置一次。
- 概览的实时趋势图已去掉，统计口径仍是审计保留记录；客户端别名从日志页挪到 规则设置 › 客户端；"高级替换规则"面板已删除（原生不支持）；域名学习页新增学习明细。
- 配置包不再附带 `ui/ilog/`；包不管理已部署机器的 `ui/`，已有的 `ui/ilog/` 不受影响。

### 升级后要手动做的事

`webinfo/requeryconfig.json` 是用户数据，包不会改它。新配置里 `cache_cnmihomo` 缓存也要随 requery 保存和清空，请在已有文件里手动补两行（各放在对应数组里，与相邻条目格式相同）：

- `url_actions.save_rules` 加：`"http://127.0.0.1:9099/plugins/cache_cnmihomo/save",`
- `url_actions.flush_rules` 加：`"http://127.0.0.1:9099/plugins/cache_cnmihomo/flush",`

端口照抄已有条目即可：requery 把主机为 `127.0.0.1`、`localhost`、`0.0.0.0`、`::`、`::1` 的 URL 一律当作本机 API，在程序内部执行，不看 URL 里的端口
（WebUI 端口设置改过 API 端口也不影响）。升级后可触发一次 requery，用 `GET /plugins/requery/status` 核对它正常结束。

### 可删除的旧文件（升级并验证后；程序不读也不删）

- `ui/ilog/`、`ui/ron/`、`ui/modify instruction`（旧页面，不用时）；
- 外观：`webinfo/appearance_settings.json`、`appearance_text_settings.json`、`appearance_button_settings.json`、`panel_background.bin`、`panel_background_history/`；
- Go 时代文件：`webinfo/upstream_overrides.json`、`config_overrides.json`、`domain_generation_settings.json`、`config_update_state.json`（Go 版的包状态；rtdns 用 `rust_config_update_state.json`）；
- v0.2.2 起不再读取的旧学习名单：`gen/*list.txt`、`gen/*rule.txt`（**不含** `gen/top_domains.txt`，requery 用它；`gen/route_learner.json` 是现行学习状态，也要保留）。
- `rule/switchN.txt` **建议保留**到确定不再需要降级为止（降级到 v0.2.2 要靠它）。

### 手动降级到 v0.2.2

先停服务，再从 `webinfo/settings.json` 删掉 `flags` 键（v0.2.2 不认识它，会拒绝启动），换回 v0.2.2 程序与 schema 2 的配置包。
`rule/switchN.txt` 仍在，但取值停留在迁移那一刻：降级前若在新程序里改过开关，需要手动把对应文件改成 `A`/`B`（对应关系见 [native-feature-flags](contracts/native-feature-flags.md) 的迁移表）。
删除 `settings.json` 或其 `flags` 键会让下一次启动重新迁移（旧的）开关文件。


## v0.2.2 升级说明（配置 schema 2，2026-10-23，路由学习重做）

- 新程序要求 `rust-config-schema-2` 的配置包；旧 schema 1 的配置（`domain_output` 带 `target_mapper`）在新程序里加载失败。
  经在线更新升级时程序与配置包一起由 update-guard 替换（失败则程序、配置文件、包状态一起回滚）；手动替换程序时必须同时更新配置包。
- 学习名单从零开始：旧的 `gen/*list.txt`、`gen/*rule.txt` 不再被读取，可手动删除；新的学习状态写在 `gen/route_learner.json`（见 [native-route-learner](contracts/native-route-learner.md)）。
- requery 不再清空学习名单。
- 用户数据不动：`webinfo/`（含 `settings.json`、专属分流组 JSON）、`rule/`、`sub_config/special_groups.yaml`（程序生成）都不在包的托管范围内。

已部署的 `v0.2.0` 及之后的 rtdns 会在系统设置里看到新版本，点“立即更新”走带自动回滚的事务（见契约文档）。

## 生产从 rust-v0.1.4 手动替换到 v0.2.0

`rust-v0.1.4` 是手动部署的，内置的更新地址指向旧仓库，且旧仓库从未发布过 `rust-v*`，所以它**不会**自己升级到 `v0.2.0`，需要手动换一次。
换成 `v0.2.0` 之后，在线更新只来自 `jasonxtt/rtdns`。配置、规则、缓存等数据都不用动（`v0.2.0` 是仅程序改动的发布，配置包标识不变）。

1. 把 `rtdns-linux-amd64.tar.gz` 与清单传到服务器的临时目录，核对 sha256（同上第 3 步），解出 `rtdns`，
   并用生产配置预检：

   ```bash
   ./rtdns version
   ./rtdns check -d /cus/rtdns -c config_custom.yaml
   ```

   `check` 必须报告 `gaps=0 cascades=0 globals=0`。
2. 备份旧程序（留着回退用），把新程序放到同目录再原子改名：

   ```bash
   cp -a /usr/local/bin/rtdns /usr/local/bin/rtdns.rust-v0.1.4
   install -m 0755 ./rtdns /usr/local/bin/rtdns.new
   mv /usr/local/bin/rtdns.new /usr/local/bin/rtdns
   ```
3. 重启并验证：

   ```bash
   systemctl restart rtdns
   systemctl is-active rtdns
   curl -s http://127.0.0.1:9099/api/v1/system/health   # "version" 应为 v0.2.0，"ready" 应为 true
   dig @127.0.0.1 example.com +short                     # 能解析
   ```

   在 WebUI 系统设置里点“检查更新”：应显示当前 `v0.2.0`，发布页指向 `jasonxtt/rtdns`，没有可用更新（还没有更高的发布时）。
   `update/installed.json` 此时不存在是正常的（它只记录经更新器安装的版本），第一次在线更新成功后才会出现。
4. 回退（任何一步异常）：

   ```bash
   systemctl stop rtdns
   mv /usr/local/bin/rtdns.rust-v0.1.4 /usr/local/bin/rtdns
   systemctl start rtdns
   ```

   旧程序用同一份配置与数据即可运行；`v0.2.0` 没有改持久化格式。
5. 确认稳定后可删除 `rtdns.rust-v0.1.4`。

## 不属于本流程

- Go 版 MosDNS 的发布、`/Users/tom/github/mosdns` 仓库、旧 Go→Rust 切换工具（`scripts/cutover/`、`tools/cutover/`）：
  这些工具仍按旧程序名和 `rust-v*` 版本格式编写，**不适用于 `v0.2.0` 及之后的程序**；生产已于 2026-10-21 完成切换，它们只作历史与回滚参考。
