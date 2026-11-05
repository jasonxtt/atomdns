# socks5_proxy / ecs_address

可以在 WebUI 修改的两个设置对象。其它插件用 `$tag` 引用它们，修改后所有引用处立即生效，不需要重启。

**类型**：设置对象　**可在 WebUI 调整**：是（上游设置 › 代理与 ECS）

## 示例

```yaml
- tag: main_proxy
  type: socks5_proxy
  args:
    addr: ""                  # 留空表示不使用代理

- tag: client_ecs
  type: ecs_address
  args:
    addr: "203.0.113.1"
```

## 参数

| 插件 | 参数 | 说明 |
| --- | --- | --- |
| `socks5_proxy` | `addr` | SOCKS5 代理地址，如 `192.0.2.9:1080`；不支持用户名密码；留空即直连 |
| `ecs_address` | `addr` | ECS 使用的 IP 地址 |

## 引用

| 引用方 | 写法 |
| --- | --- |
| [`aliapi`](/plugins/forward#aliapi) | `proxy: $main_proxy`，上游项加 `use_proxy: true` |
| 规则订阅（`sd_set_light`、`si_set`、`adguard_rule`） | `proxy: $main_proxy`，用于下载规则 |
| [`ecs` 简写](/plugins/ecs_handler#简写) | `exec: ecs $client_ecs` |

## 说明

- WebUI 修改的值保存在 `webinfo/settings.json`，YAML 保持默认值。
- 代理只用于 TCP、DoT、DoH 上游和规则下载，UDP 上游不走代理。代理连接失败时该上游视为失败，不会回退直连。
