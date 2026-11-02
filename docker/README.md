# AtomDNS

**AtomDNS** is a Rust rewrite of [MosDNS](https://github.com/yyysuo/mosdns): a DNS splitter for home networks, side routers and transparent-proxy setups, with a Vue WebUI. It keeps MosDNS's configuration (YAML, `sequence`/plugin semantics), routing results, WebUI/API workflows and persistent formats.

**AtomDNS** 是 [MosDNS](https://github.com/yyysuo/mosdns) 的纯 Rust 重写：面向家庭网络、旁路由和透明代理的 DNS 分流程序，带 WebUI。配置（YAML、`sequence`/插件语义）、分流结果、WebUI/API 工作流和持久化格式与 MosDNS 保持一致。

源码与文档 / Source & docs: <https://github.com/jasonxtt/atomdns> · 完整部署说明 / full guide: [docs/rust/docker.md](https://github.com/jasonxtt/atomdns/blob/main/docs/rust/docker.md)

## 标签与平台 / Tags and platforms

| 标签 | 说明 |
| --- | --- |
| `latest` | 最新正式版 / latest release |
| `vX.Y.Z` | 与 GitHub Release 同名的固定版本 / pinned release (e.g. `v0.6.2`) |

平台：`linux/amd64`、`linux/arm64`、`linux/arm/v7`（Debian slim，glibc 程序）。

## 快速开始 / Quick start

```bash
mkdir -p atomdns-data
docker run -d --name atomdns --restart unless-stopped --network host \
  -e TZ=Asia/Shanghai \
  -v "$PWD/atomdns-data:/cus/atomdns" \
  jasonxtt/atomdns:latest
```

WebUI：`http://<主机>:9099/`。

- 配置目录是容器内的 `/cus/atomdns`，挂载到宿主机目录即可持久化（配置、规则、缓存转储、学习数据、查询日志）。
- **空目录首次启动**会用镜像内置的全新安装包初始化（含 WebUI），不需要联网。目录里有别的文件却没有 `config_custom.yaml` 时拒绝启动，不会覆盖任何文件。
- 已有配置目录（例如从裸机 `/cus/atomdns` 迁来）直接挂载即可，保留原有数据。
- 默认配置的上游指向宿主机上的伴生服务（`127.0.0.1:6666`、`:1053`、`:2222`）。host 网络下可直接使用；bridge 网络下容器内的 `127.0.0.1` 是容器自己，要改成宿主机地址或伴生容器的服务名，否则查询返回 SERVFAIL。
- host 网络下，默认配置的**所有**监听端口（53、2222、3066、3077、3099、3111、3333、4444、7777、8888，以及 API 9099）都要求宿主机上空闲；任何一个被占用，容器会以 `TCP bind failed: Address in use` 退出重启。端口冲突时改用 bridge 网络（只映射 53 和 9099），或在配置里改掉冲突的 `listen`。

## Docker Compose

### host 网络（Linux 推荐）

容器共享宿主机网络：默认配置的 `127.0.0.1` 伴生服务可直接访问，配置里所有监听端口（`:53`、专属分流组的 `:3066`、`:3077` …）无需逐个映射，查询日志里是真实的客户端地址。宿主机的 53 端口必须空闲。

```yaml
services:
  atomdns:
    image: jasonxtt/atomdns:latest
    container_name: atomdns
    restart: unless-stopped
    network_mode: host
    environment:
      TZ: Asia/Shanghai
    volumes:
      - ./atomdns-data:/cus/atomdns
```

### bridge 网络

适用于 Docker Desktop、Unraid/NAS 的自定义网络，或必须映射端口的场景。额外监听的端口（专属分流组等）要自己加映射。

```yaml
services:
  atomdns:
    image: jasonxtt/atomdns:latest
    container_name: atomdns
    restart: unless-stopped
    environment:
      TZ: Asia/Shanghai
    ports:
      - "53:53/udp"
      - "53:53/tcp"
      - "9099:9099/tcp"
    volumes:
      - ./atomdns-data:/cus/atomdns
    # 伴生服务在 Docker 宿主机上时 / a companion on the Docker host:
    # extra_hosts:
    #   - "host.docker.internal:host-gateway"
```

## 升级 / Upgrading

容器版**关闭了 WebUI 里的在线更新**，升级就是拉新镜像并重建容器：

```bash
docker compose pull && docker compose up -d
# 或 / or
docker pull jasonxtt/atomdns:latest && docker rm -f atomdns && docker run ...
```

镜像带着在线更新原本会带来的全部内容，新容器启动时会让挂载目录跟上镜像：

1. 配置 schema 落后时应用内置配置包（只替换托管的 `config_custom.yaml`、`sub_config/*.yaml`，不碰 `webinfo/settings.json` 等用户数据），替换前的旧文件备份在 `backup/container-schema-…/`。
2. `ui/main/` 版本与镜像不同时整体替换为镜像内置的界面；删掉 `ui/main/` 就是纯后端，镜像不会再创建它。
3. 规则、缓存、学习数据、`settings.json` 原样保留。

## 常用命令 / Commands

```bash
docker run --rm jasonxtt/atomdns version
docker run --rm -v "$PWD/atomdns-data:/cus/atomdns" jasonxtt/atomdns \
  check -d /cus/atomdns -c config_custom.yaml        # 只检查配置 / validate only
docker logs -f atomdns
```

## 端口 / Ports

| 端口 | 用途 |
| --- | --- |
| 53/udp, 53/tcp | DNS |
| 9099/tcp | WebUI 与管理 API / WebUI and management API |

其余端口由配置决定（专属分流组等）。容器以 root 运行，监听 53 不需要额外能力。

## 许可 / License

GPL-3.0，见 <https://github.com/jasonxtt/atomdns/blob/main/LICENSE>。
