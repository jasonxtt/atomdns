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

   产物：`release/rust/rtdns-linux-amd64.tar.gz`、`release/rust/rtdns-update-manifest.json`。
   脚本会拒绝非 `vX.Y.Z` 的 tag，要求构建出的程序自报版本等于 tag，并核对 `scripts/rust-release-metadata.json`
   与 `update/mod.rs` 里的配置包标识和两个资产名。脚本只构建，不发布。
3. 核对：

   ```bash
   cd release/rust
   sha256sum rtdns-linux-amd64.tar.gz
   jq '.version, .artifacts' rtdns-update-manifest.json
   mkdir -p /tmp/rtdns-verify && tar -xzf rtdns-linux-amd64.tar.gz -C /tmp/rtdns-verify
   /tmp/rtdns-verify/rtdns version
   ```

   sha256 必须与清单一致，`version` 必须等于 tag。
4. 在 GitHub 建 Release（tag 与 Release 同名，不是 draft、不是 prerelease），把上面两个文件作为资产上传，例如：

   ```bash
   gh release create v0.2.0 release/rust/rtdns-linux-amd64.tar.gz release/rust/rtdns-update-manifest.json \
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
5. **建 Release**，一次上传三个资产：程序包、清单、`config_rust_up.zip`（文件名必须与 `config.url` 末段一致）。
   Release 建好后，用 `curl -fsSL <config.url> | sha256sum` 再核对一次与元数据一致。更新器只在 `applied_config_schema < required_config_schema` 时下载配置包。

## 配置 schema 3（功能开关，M2-1，2026-10-23；随 v0.3.0 发布）

- 新程序要求 `rust-config-schema-3` 的配置包：`switch1`–`switch17` 换成 `sub_config/flags.yaml` 里的 13 个具名开关（`feature_flag`），包删除 `sub_config/switch.yaml`。
  旧配置里的 `switchN` 插件或匹配写法在新程序里加载失败；手动替换程序时必须同时更新配置包。
- 开关值改存 `webinfo/settings.json` 的 `flags` 段。首次正常启动时从 `rule/switchN.txt` 迁移一次（在线更新试运行期间不写，回滚安全），旧文件保留但不再读取。
- 旧接口 `/plugins/switchN/show|post` 已删除；现 WebUI 的开关页在 UI 重做前不可用，可用 `GET/POST/DELETE /api/v1/flags[/{名字}]`（见 [native-feature-flags](contracts/native-feature-flags.md)）。
- **手动降级到 v0.2.2**：先从 `webinfo/settings.json` 删掉 `flags` 键（v0.2.2 不认识它，会拒绝启动），并换回 schema 2 的配置包。
  旧的 `rule/switchN.txt` 仍在，但取值停留在迁移那一刻，降级前若在新程序里改过开关，需要手动把对应文件改成 `A`/`B`。

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
