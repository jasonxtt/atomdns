# cache

缓存应答。命中时直接返回，跳过后面的规则；未命中时，后面的规则得到应答后自动存入。

**类型**：执行器　**可在 WebUI 调整**：是（缓存设置）

## 示例

```yaml
- tag: cache_main
  type: cache
  args:
    label: 主缓存
    size: 100000
    lazy_cache_ttl: 604800
    dump_file: cache/cache_main.dump
    dump_interval: 21600
```

```yaml
- tag: main
  type: sequence
  args:
    - exec: $cache_main          # 放在转发之前
    - exec: $forward_remote
```

## 参数

| 参数 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `size` | 整数 | `1024` | 最多缓存的条数，超出后淘汰较少使用的 |
| `lazy_cache_ttl` | 秒 | `0` | 过期保留时长：应答过期后仍先返回旧结果，同时在后台刷新。`0` 为不保留 |
| `dump_file` | 路径 | — | 落盘文件，重启后恢复缓存 |
| `dump_interval` | 秒 | `600` | 落盘间隔；程序退出时也会写入 |
| `enable_ecs` | 布尔 | `false` | 带 ECS 的查询按 ECS 网段分开缓存 |
| `label` | 字符串 | — | WebUI 显示的名称，不影响缓存内容 |

## 工作方式

- `cache` 包住它所在序列中**后面的所有规则**。命中后跳过它们，序列在 `cache` 所在层级继续。
- 开启 `lazy_cache_ttl` 后，过期应答以 TTL 5 秒返回给客户端，同时后台刷新，刷新完成后替换缓存。
- 在 WebUI 修改容量、过期保留时长或落盘间隔后，该缓存以空缓存重新开始。

## 管理

「规则设置 › 缓存设置」可以查看条目、清空缓存。也可以调用接口：

| 接口 | 用途 |
| --- | --- |
| `GET /plugins/<tag>/flush` | 清空 |
| `GET /plugins/<tag>/save` | 立即落盘 |
| `GET /plugins/<tag>/show` | 列出条目 |

## 简写

```yaml
- exec: cache 4096     # 私有缓存，只能指定容量，不支持过期保留
```

## 与 MosDNS 的差异

- `exclude_ip` 目前不支持，写了非空值会报错。
- 落盘文件格式与 MosDNS 相同，但规则改变后旧缓存仍会命中，需要立即生效请清空缓存。
