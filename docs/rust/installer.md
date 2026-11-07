# 一键安装（AtomDNS + 定制版 sing-box）

`scripts/install.sh` 会在一台机器上一次装好 AtomDNS 和定制版 sing-box，并把两边配好：AtomDNS 做唯一一次分流决策，sing-box 用 FakeIP 接住代理流量，按 AtomDNS 指定的出口转发。原理见 [native-singbox-integration.md](contracts/native-singbox-integration.md)。

## 用法

```sh
curl -fsSL https://raw.githubusercontent.com/DefaWong/atomdns/main/scripts/install.sh | sh
```

Alpine 默认没有 curl 和 bash，可以用自带的 wget 和 sh：

```sh
wget -qO- https://raw.githubusercontent.com/DefaWong/atomdns/main/scripts/install.sh | sh
```

脚本是 POSIX `sh`，`| bash` 也能用。必须以 root 运行。

| 环境变量 | 默认值 | 说明 |
| --- | --- | --- |
| `ATOMDNS_RELEASE` | `latest` | 要安装的 Release tag |
| `ATOMDNS_REPO` | `DefaWong/atomdns` | 下载 Release 的仓库 |
| `ATOMDNS_ASSETS` | 空 | 本地目录，放好全部资产后不再下载（测试用） |
| `ATOMDNS_ROOT` | `/cus/atomdns` | AtomDNS 配置根 |
| `TPROXY_PORT` | `7895` | sing-box TProxy 端口 |

## 支持的系统

- x86_64（amd64）。两个系统用的都是静态 musl 版 AtomDNS，以及 CGO 关闭的静态 sing-box。
- Alpine（OpenRC）：用 apk 安装依赖。
- Debian 系（Debian、Ubuntu 等，systemd）：用 apt 安装依赖。

UDP 53 端口被其它程序占用（systemd-resolved 的 stub、dnsmasq 等）时，脚本会退出，请先释放端口再重跑。

## 装了什么

| 内容 | 位置 |
| --- | --- |
| 程序 | `/usr/local/bin/atomdns`、`/usr/local/bin/sing-box` |
| AtomDNS 配置 | `/cus/atomdns`（来自 `config_rust.zip`，`ui/main` 换成本 Release 的 WebUI） |
| AtomDNS 设置 | `/cus/atomdns/webinfo/settings.json`（权限 600）：`main_proxy` 指向 sing-box 的 mixed 入口，sing-box API 的地址和密钥 |
| sing-box 配置 | `/etc/sing-box/config.json`（权限 600），状态在 `/var/lib/sing-box` |
| TProxy 规则 | `/usr/local/lib/atomdns/tproxy.sh`，由 sing-box 服务在启动时加载、停止时撤销 |
| 服务 | OpenRC：`/etc/init.d/{sing-box,atomdns}`；systemd：`/etc/systemd/system/{sing-box,atomdns}.service`；都设为开机自启 |

sing-box API 的密钥是安装时随机生成的，同时写进 sing-box 配置和 AtomDNS 设置。

端口：

| 端口 | 程序 | 用途 |
| --- | --- | --- |
| `:53` | AtomDNS | DNS |
| `:9099` | AtomDNS | WebUI 和 API |
| `127.0.0.1:6666` | sing-box | FakeIP DNS（AtomDNS 的 `nocnfake` 上游） |
| `127.0.0.1:9666` | sing-box | mixed 代理（AtomDNS 的 `main_proxy`） |
| `127.0.0.1:9090` | sing-box | Clash API（带密钥） |
| `:7895` | sing-box | TProxy 入口 |

新装的 sing-box 配置里，`proxy` 分组只有 `direct`。在 WebUI 的 sing-box 页面添加节点或订阅之后，分组会按 AtomDNS 的节点列表重建。

## 主路由怎么配

先给这台机器固定 IP（静态地址或 DHCP 保留）：客户端的 DNS 和 FakeIP 路由都指向它，IP 变了两者都会失效。

1. 让客户端用这台机器做 DNS 服务器。
2. 把 FakeIP 网段 `198.18.0.0/15` 和 `fc00::/18` 路由到这台机器，可以用静态路由，也可以用策略路由。

到达这台机器、目标是 FakeIP 的 TCP/UDP 流量，会被 nftables 的 TProxy 交给 sing-box：

- `table inet atomdns_tproxy`，打 fwmark `0x1d`。
- 策略路由走表 `233`。

sing-box 按 FakeIP 反查出域名和绑定的出口：有绑定就直接走这个出口，没有就走 `proxy` 分组。本机自己发出的流量不经过 TProxy。

## 重复运行（升级）

再跑一次脚本会：

- 更新两个程序、WebUI、服务文件和 TProxy 脚本。
- 保留 `/cus/atomdns` 下已有的配置、`webinfo/settings.json` 和 `/etc/sing-box/config.json`，sing-box API 的密钥继续沿用。

AtomDNS 的在线更新源是 `DefaWong/atomdns` 的 Release（`update/mod.rs`）。在线更新只更新 AtomDNS 和 WebUI，sing-box 要重跑安装脚本才会更新。

## 卸载

```sh
# Alpine
rc-service atomdns stop; rc-service sing-box stop
rc-update del atomdns default; rc-update del sing-box default
rm -f /etc/init.d/atomdns /etc/init.d/sing-box
# Debian 系
systemctl disable --now atomdns sing-box
rm -f /etc/systemd/system/atomdns.service /etc/systemd/system/sing-box.service && systemctl daemon-reload
# 两种系统都要执行
rm -rf /usr/local/bin/atomdns /usr/local/bin/sing-box /usr/local/lib/atomdns
# 配置和数据，确认不再需要后再删
rm -rf /cus/atomdns /etc/sing-box /var/lib/sing-box
```

停止 sing-box 服务时 TProxy 规则就已经撤销了。

## Release 资产

脚本从 `https://github.com/<repo>/releases/<latest|download/tag>/` 下载以下文件，按 `SHA256SUMS` 校验。这些文件都由 `release-build.yml` 一次构建完成：

| 资产 | 来源 |
| --- | --- |
| `atomdns-linux-amd64-musl.tar.gz` | `scripts/build-rust-release.sh`（`VARIANTS="amd64 amd64-v3 amd64-musl"`） |
| `atomdns-webui.tar.gz` | 同上 |
| `config_rust.zip` | 由 `scripts/installer-assets.json` 的 `config_full` 指定，下载后按其中的 sha256 校验 |
| `sing-box-linux-amd64.tar.gz` | 按 `scripts/installer-assets.json` 的 `singbox` 固定的仓库和提交检出定制版 sing-box，用该仓库 `go.mod` 里的 Go 版本编译 |
| `SHA256SUMS` | 由 `scripts/package-installer-assets.sh` 生成 |

同一个构建产物里还有在线更新要用的 `atomdns-linux-amd64.tar.gz`、`-amd64-v3.tar.gz` 和 `atomdns-update-manifest.json`。这些文件和上表的资产一起放在名为 `release-<tag>` 的 artifact 里，直接全部上传到 Release 即可。

### 更新 sing-box

定制版 sing-box 在 `DefaWong/sing-box` 的默认分支 `atomdns` 上。这个分支只追加提交，不 rebase，所以固定下来的提交号一直有效。

1. 在 sing-box 仓库提交并推送到 `atomdns` 分支。
2. 把新的完整提交号写进 `scripts/installer-assets.json` 的 `singbox.commit`；如果 sing-box 的基础版本变了，同时改 `singbox.version`。然后提交 atomdns。
3. 发版时运行 `release-build.yml`。

sing-box 的版本号形如 `<singbox.version>-atomdns.<提交号前 10 位>`，例如 `1.14.4-reF1nd-atomdns.1c29dbce59`。

### 本地打包

打包脚本要求 `SINGBOX_SRC` 正好检出在固定的提交上，并且没有未提交的改动；`CONFIG_ZIP` 的 sha256 也必须和固定值一致：

```bash
ATOMDNS_MUSL_TGZ=release/rust/atomdns-linux-amd64-musl.tar.gz \
WEBUI_TGZ=release/rust/atomdns-webui.tar.gz \
CONFIG_ZIP=/path/to/config_rust.zip \
SINGBOX_SRC=/path/to/sing-box \
  scripts/package-installer-assets.sh release/installer
```

打包脚本只构建不发布。在 `DefaWong/atomdns` 建 Release 属于对外发布，需要先确认。
