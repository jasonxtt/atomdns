# fallback

主备两路执行：先跑主路，主路在阈值时间内没有结果或失败时，启动备路，采用先到的有效应答。

**类型**：执行器

## 示例

```yaml
- tag: remote_with_backup
  type: fallback
  args:
    primary: forward_remote
    secondary: forward_backup
    threshold: 500
    always_standby: false
```

## 参数

| 参数 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `primary` | tag | 必填 | 主路，可以是序列或执行器插件（不带 `$`） |
| `secondary` | tag | 必填 | 备路 |
| `threshold` | 毫秒 | `500` | 主路超过这个时间没有应答就启动备路 |
| `always_standby` | 布尔 | `false` | 主路和备路同时启动，主路超时后直接用备路的结果 |

## 说明

- 主路失败（没有应答或出错）时立即启动备路，不必等到阈值。
- 主路与备路各自独立处理查询，互不影响。
