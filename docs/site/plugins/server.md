# udp_server / tcp_server

监听端口接收 DNS 查询，交给入口序列处理。同一端口通常 UDP 和 TCP 各定义一个。

**类型**：服务器

## 示例

```yaml
- tag: udp_main
  type: udp_server
  args:
    entry: main
    listen: ":53"
    enable_audit: true

- tag: tcp_main
  type: tcp_server
  args:
    entry: main
    listen: ":53"
    enable_audit: true
    idle_timeout: 720
```

## 参数

| 参数 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `entry` | 字符串 | 必填 | 入口序列的 tag |
| `listen` | 地址 | 必填 | 监听地址，`:53` 表示所有地址的 53 端口，也可写 `127.0.0.1:53` |
| `enable_audit` | 布尔 | `false` | 是否记入查询日志与概览统计 |
| `idle_timeout` | 秒 | `10` | 仅 `tcp_server`：空闲连接保持时长 |

## 说明

- 只给内部使用的端口可以不开 `enable_audit`，避免这些流量混进日志与统计。
- 默认配置只监听 53（DNS），WebUI 与管理接口在 `api.http` 的 9099。
- 查询日志的保留条数在「系统管理 › 审计日志」设置。

## 与 MosDNS 的差异

不支持 `http_server`、`quic_server`，以及 TLS 证书相关参数。
