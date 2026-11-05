# ecs_handler

为发往上游的查询附加 ECS（EDNS Client Subnet），让上游按指定网段返回就近的地址。

**类型**：执行器

## 示例

```yaml
- tag: ecs_remote
  type: ecs_handler
  args:
    forward: false
    send: true
    mask4: 24
    mask6: 48

- tag: main
  type: sequence
  args:
    - exec: $ecs_remote
    - exec: $forward_remote
```

## 参数

| 参数 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `forward` | 布尔 | `false` | 保留客户端查询里已有的 ECS |
| `send` | 布尔 | `false` | 没有 ECS 时，用客户端 IP 生成 |
| `preset` | IP | — | 固定使用这个地址，可写 IP 或引用 [`ecs_address`](/plugins/settings-objects)（`$client_ecs`） |
| `mask4` | 整数 | `24` | IPv4 网段长度，0–32 |
| `mask6` | 整数 | `48` | IPv6 网段长度，0–128 |

选择顺序：客户端已有的 ECS（`forward`）→ `preset` → 客户端 IP（`send`）→ 不附加。

## 简写

```yaml
- exec: ecs 203.0.113.0        # 固定地址
- exec: ecs $client_ecs        # 引用可在 WebUI 修改的地址
```

## 说明

带 ECS 的查询默认不进缓存；要缓存它们，给对应的 [cache](/plugins/cache) 打开 `enable_ecs`。
