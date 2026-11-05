<p align="center"><img src="https://raw.githubusercontent.com/jasonxtt/atomdns/main/docs/assets/atomdns-banner.jpg" alt="AtomDNS" width="520"></p>

<p align="center"><b>用 Rust 重写的 MosDNS：带完整 WebUI 的家庭网络 DNS 分流器</b><br>
A Rust rewrite of MosDNS — a DNS splitter for home networks, with a full WebUI.</p>

<p align="center">
  <a href="https://jasonxtt.github.io/atomdns/">📖 使用文档</a> ·
  <a href="https://github.com/jasonxtt/atomdns">GitHub</a> ·
  <a href="https://github.com/jasonxtt/atomdns/releases">更新日志</a>
</p>

---

## 特性

- **精细分流**：国内外分流，FakeIP / RealIP 两种模式，专属分流组把指定域名交给专属上游与独立缓存
- **路由学习**：自动记住直连、代理、无 IPv4 / IPv6 的域名，按时间过期
- **在线规则**：订阅 SRS、MRS、dat、Clash、文本规则，保存即生效
- **查询审计**：每条查询的最终分流、实际上游、耗时一目了然
- **开箱即用**：首次启动自动写入完整默认配置和 WebUI，无需联网

![概览](https://jasonxtt.github.io/atomdns/screenshots/overview.jpg)

## 快速开始

```bash
docker run -d --name atomdns --restart unless-stopped --network host \
  -e TZ=Asia/Shanghai \
  -v "$PWD/atomdns-data:/cus/atomdns" \
  jasonxtt/atomdns:latest
```

打开 `http://<主机 IP>:9099/` 进入 WebUI，把路由器下发的 DNS 改成这台主机的 IP 即可使用。

## Docker Compose

### host 网络（推荐）

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

容器共享宿主机网络：配置里的所有端口直接生效，查询日志显示真实的客户端地址，宿主机上的代理软件可以直接用 `127.0.0.1` 访问。

### bridge 网络

适合 Docker Desktop、NAS 自定义网络等必须映射端口的环境。

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
    # 代理软件运行在宿主机上时：
    # extra_hosts:
    #   - "host.docker.internal:host-gateway"
```

bridge 网络下需要注意：

- 容器里的 `127.0.0.1` 是容器自己。默认配置中指向代理软件的上游（FakeIP 上游 `127.0.0.1:6666`、国内 FakeIP 上游 `127.0.0.1:1053`）要在 WebUI「上游设置」改成宿主机地址，例如 `host.docker.internal`。
- 查询日志里的客户端地址是 Docker 网关地址，按客户端分流不可用。
- 用到的其它端口（见下表）要自己加映射。

## 数据目录

容器内的配置目录是 `/cus/atomdns`，挂载出来即可持久化配置、规则、缓存、学习结果和查询日志。

| 情况 | 行为 |
| --- | --- |
| 空目录 | 首次启动写入完整默认配置和 WebUI |
| 已有配置（含 `config_custom.yaml`） | 直接使用，例如从裸机的 `/cus/atomdns` 迁来 |
| 有其它文件但没有 `config_custom.yaml` | 拒绝启动，不覆盖任何文件 |

## 端口

| 端口 | 用途 | bridge 下是否需要映射 |
| --- | --- | --- |
| 53 | DNS | 需要 |
| 9099 | WebUI 与管理接口 | 需要 |
| 3077 / 3099 / 3111 | 给 Stash、ClashMi、sing-box 回家用的 DNS | 用到时 |
| 7777 / 8888 | 给代理软件解析节点域名、给 sing-box 使用的国内 DNS | 用到时 |
| 4444 | 带 ECS 的国外查询 | 用到时 |
| 2222 / 3333 / 3066 | 内部使用（国内查询、国外查询、定时刷新） | 不需要 |
| 专属分流组的自定义端口 | 只走该组的 DNS | 用到时 |

host 网络下，以上端口在宿主机上都必须空闲，任何一个被占用，容器会报 `Address in use` 并不断重启。
这时可以停掉占用端口的服务（常见的是 `systemd-resolved` 占用 53），或改用 bridge 网络。

## 升级

容器版不使用 WebUI 里的在线更新，拉取新镜像并重建容器即可：

```bash
docker compose pull && docker compose up -d
```

新容器启动时会自动让数据目录跟上新版本：

- 配置结构有变化时，替换配置包管理的 `config_custom.yaml` 与 `sub_config/*.yaml`，旧文件备份到 `backup/` 下；
- `ui/main/` 替换为新版界面（删除 `ui/main/` 即为纯后端，之后不会再创建）；
- 规则、缓存、学习结果、WebUI 里的设置都原样保留。

需要回退时换回旧版本标签即可。配置结构只升不降，跨配置版本回退前请先看发布说明。

## 标签与平台

| 标签 | 说明 |
| --- | --- |
| `latest` | 最新正式版 |
| `vX.Y.Z` | 指定版本，与 GitHub Release 同名 |

平台：`linux/amd64`、`linux/arm64`、`linux/arm/v7`。

## 常用命令

```bash
docker logs -f atomdns                       # 查看日志
docker exec atomdns atomdns version          # 查看版本
docker run --rm -v "$PWD/atomdns-data:/cus/atomdns" jasonxtt/atomdns:latest \
  check -d /cus/atomdns -c config_custom.yaml   # 只检查配置
```

## 更多

完整说明见 **[使用文档](https://jasonxtt.github.io/atomdns/)**：WebUI 导览、运行模式、专属分流组、每个插件的参数等。

许可证：[GPL-3.0](https://github.com/jasonxtt/atomdns/blob/main/LICENSE)。致谢 [MosDNS](https://github.com/IrineSistiana/mosdns) 与 [yyysuo/mosdns](https://github.com/yyysuo/mosdns)。
