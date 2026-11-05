# 快速开始

五分钟把 AtomDNS 跑起来，并让局域网设备用上它。

## Docker 安装

推荐方式。镜像支持 amd64 / arm64 / armv7。

```bash
docker run -d --name atomdns --restart unless-stopped --network host \
  -v "$PWD/atomdns-data:/cus/atomdns" jasonxtt/atomdns:latest
```

首次启动会把默认配置写入挂载目录 `atomdns-data`。

::: tip 端口
默认监听 `53`（DNS）和 `9099`（WebUI 与 API），两者在宿主机上都必须空闲。
端口被占用时可以改用 bridge 网络，只映射这两个端口。
:::

## 二进制安装

1. 从 [Releases](https://github.com/jasonxtt/atomdns/releases) 下载对应架构的程序包和全新安装包 `config_rust.zip`。
2. 安装程序并解压配置：

   ```bash
   tar -xzf atomdns-linux-amd64.tar.gz
   install -m 755 atomdns /usr/local/bin/atomdns
   mkdir -p /cus/atomdns && unzip config_rust.zip -d /cus/atomdns
   ```

3. 写入 `/etc/systemd/system/atomdns.service`：

   ```ini
   [Unit]
   Description=AtomDNS
   After=network-online.target

   [Service]
   ExecStart=/usr/local/bin/atomdns start -d /cus/atomdns -c config_custom.yaml
   Restart=always

   [Install]
   WantedBy=multi-user.target
   ```

4. 启动：

   ```bash
   systemctl daemon-reload && systemctl enable --now atomdns
   ```

::: details 该选哪个程序包？
| 程序包 | 适用 |
| --- | --- |
| `amd64` | 一般 x86_64 机器，支持在线更新 |
| `amd64-v3` | 支持 AVX2 的较新 x86_64 |
| `amd64-musl` | Alpine 等 musl 系统 |
| `arm64` / `armv7` | ARM 设备（无后缀为静态版，`-gnu` 为 glibc 版） |
:::

## 验证

```bash
dig @127.0.0.1 www.baidu.com +short
```

能返回 IP 就说明 DNS 已经在工作。再打开 `http://<主机 IP>:9099/`，概览页应能看到刚才的查询。

## 让设备用上它

把路由器 DHCP 下发的 DNS 改成 AtomDNS 所在主机的 IP，设备重新联网后即生效。

::: warning 旁路由 / 透明代理
使用 FakeIP 模式时，需要在「上游设置」把「国外 FakeIP 上游」指向代理软件的 DNS 端口，
例如 `udp://192.0.2.1:6666`（换成你的代理软件地址）。详见 [运行模式](/guide/modes)。
:::

## 下一步

- 认识界面：[WebUI 导览](/guide/webui)
- 了解默认配置怎样分流：[分流流程](/guide/flow)
- 按需调整上游：[forward 插件](/plugins/forward)
