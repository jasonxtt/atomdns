# flow_setter / tag_setter

设置查询日志里显示的分流信息，不影响解析结果。用来让日志更易读。

**类型**：执行器

## flow_setter

记录这次查询最终走了哪个分组、哪个序列、哪个上游组：

```yaml
- exec: flow_setter group=流媒体 sequence=sequence_media upstream=forward_media
```

| 键 | 日志字段 |
| --- | --- |
| `group` | 命中的分组 |
| `sequence` | 最终处理的序列 |
| `upstream` | 最终使用的上游组 |

至少写一个键。插件写法用 `matched_group`、`final_sequence`、`final_upstream` 三个参数：

```yaml
- tag: flow_media
  type: flow_setter
  args:
    matched_group: 流媒体
    final_upstream: forward_media
```

## tag_setter

设置日志里的分流标签：

```yaml
- exec: tag_setter sing-box国内
```

插件写法：

```yaml
- tag: tag_home
  type: tag_setter
  args:
    tag: 回家
```
