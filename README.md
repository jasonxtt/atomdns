# rtdns

**rtdns** 是 [MosDNS](https://github.com/yyysuo/mosdns) 的纯 Rust 重写：一个面向家庭网络、旁路由和透明代理环境的 DNS 分流程序，
内置 Vue WebUI，保持 MosDNS 的配置（YAML、`sequence`/插件语义）、DNS 分流结果、WebUI/API 工作流和持久化格式不变。
构建只需要 Rust 工具链（WebUI 打包用 Node），没有 Go 源码、不需要 Go 工具链。

## 与 MosDNS 的关系

- rtdns 的行为目标是 MosDNS-T（[`jasonxtt/mosdns`](https://github.com/jasonxtt/mosdns)，基于 `yyysuo/mosdns` 持续维护的分支）：
  同样的配置、同样的专属分流组、在线规则、查询日志与 WebUI，用 Rust 重新实现运行时。
  Go 版仍在 `jasonxtt/mosdns` 维护；它的安装脚本、Docker 镜像和 OpenWrt 软件源**不在本仓库**，本仓库也不提供这些发行形式。
- 本仓库从 2026-10-21 起独立，保留 `jasonxtt/mosdns` 的 `rust` 分支的完整历史。
- 以下名字沿用 MosDNS，不随项目改名：crate 名（`mosdns-*`）、`MOSDNS_*` 环境变量、`mosdns_*` 指标名、WebUI/API 里的既有字段、
  配置包标识（`channel "rust"`、`rust-config-schema-N`、`config_rust_up.zip`）。

## 主要能力

- 国内外域名分流、FakeIP / Redir-Host 两种模式；白名单、灰名单、DDNS 域名、重定向等名单维护。
- 专属分流组：命中某组的域名交给绑定的专属上游和专属缓存。
- 在线规则订阅、本地规则、查询日志与诊断、缓存管理。
- 内置 WebUI（`/`）；也可以在配置目录的 `ui/<name>/` 放外置 UI 包，见 [external-ui.md](docs/rust/external-ui.md)。
- 带自动回滚的在线更新与配置包更新，见 [native-update.md](docs/rust/contracts/native-update.md)。

## 安装与在线更新

目前只发布 **Linux amd64** 程序，没有安装脚本。

1. 从本仓库的 [Releases](https://github.com/jasonxtt/rtdns/releases) 下载 `rtdns-linux-amd64.tar.gz` 与 `rtdns-update-manifest.json`，
   用清单核对 sha256，解出 `rtdns`，放到例如 `/usr/local/bin/rtdns`。
2. 准备配置目录（例如 `/cus/rtdns`，里面是 `config_custom.yaml` 等文件；配置规则见 [config-notes.md](docs/ai/config-notes.md)）。
   从 Go 版 MosDNS 迁移的历史切换流程见 [cutover.md](docs/rust/cutover.md)（该工具按旧程序名编写，仅作参考）。
3. 用 systemd 管理，例如：

   ```ini
   [Service]
   ExecStart=/usr/local/bin/rtdns start -d /cus/rtdns -c config_custom.yaml
   Restart=always
   ```

   命令行：`rtdns start [-d <dir>] -c <config>`、`rtdns check [-d <dir>] -c <config>`（只检查配置）、`rtdns version`。
4. 在线更新：WebUI「系统设置」检查更新并“立即更新”。更新只来自 `jasonxtt/rtdns` 的 `vX.Y.Z` 发布，
   程序和（需要时的）配置包都经 sha256 校验，新程序在守护进程下试运行，不健康会自动回滚到旧程序和旧配置。

## 构建

```bash
scripts/build-rust-native.sh          # 先构建 WebUI 再构建程序，输出 release/rtdns-native
scripts/build-rust-release.sh vX.Y.Z  # 发布包与更新清单（只构建，不发布）
cargo test --workspace --no-fail-fast --manifest-path rust/Cargo.toml
```

不要同时运行前端与 Rust 构建。发版与手动替换步骤见 [release.md](docs/rust/release.md)。

## 文档

- [不使用 WebUI：持久设置、校验与生效](docs/rust/headless.md)
- [AGENTS.md](AGENTS.md)、[项目背景](docs/ai/project-context.md)、[配置说明](docs/ai/config-notes.md)
- [产品契约](docs/rust/contracts/index.md)、[验证摘要](docs/rust/validation-summary.md)
- [发版与手动替换](docs/rust/release.md)、[Go→Rust 切换（历史）](docs/rust/cutover.md)
- 与上游 MosDNS 的同步范围：[UPSTREAM_SYNC.md](UPSTREAM_SYNC.md)

## 开源协议

本项目采用 **GNU General Public License v3.0（GPL-3.0）**，见 [LICENSE](LICENSE)。使用、修改和再发布时请保留原作者及相关版权声明。

## 致谢

基于 [`yyysuo/mosdns`](https://github.com/yyysuo/mosdns) 及其维护分支 [`jasonxtt/mosdns`](https://github.com/jasonxtt/mosdns)，感谢原项目作者及所有贡献者。
