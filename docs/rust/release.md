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
5. 如果本次发布需要新的配置包（结构性改动，`required_config_schema` 与 `config_package_id` 同时提高），
   先发布配置包并把 URL 与 sha256 写进 `scripts/rust-release-metadata.json` 的 `config`，再重新生成清单；仅程序改动的发布不需要。
   配置包标识（`channel "rust"`、`rust-config-schema-N`、`config_rust_up.zip`）不随改名变化。

   **配置 schema 2（2026-10-23，路由学习重做）**：新程序要求 `rust-config-schema-2` 的配置包；旧 schema 1 的配置（`domain_output` 带 `target_mapper`）在新程序里加载失败。
   经在线更新升级时程序与配置包一起换；手动替换程序时必须同时更新配置包。旧的 `gen/*list.txt`、`gen/*rule.txt` 不再被读取，可手动删除；
   学习状态从空开始，写在 `gen/route_learner.json`（见 [native-route-learner](contracts/native-route-learner.md)）。

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
