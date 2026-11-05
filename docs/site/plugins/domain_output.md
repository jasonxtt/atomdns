# domain_output

统计查询过的域名及次数，写入文件。默认配置用它生成访问排行，也作为 [requery](/plugins/requery) 的域名来源。

**类型**：执行器

## 示例

```yaml
- tag: top_domains
  type: domain_output
  args:
    file_stat: gen/top_domains.txt
    max_entries: 50000
    dump_interval: 3600

- tag: main
  type: sequence
  args:
    - exec: $top_domains
    - exec: $forward_remote
```

## 参数

全部可选。

| 参数 | 说明 |
| --- | --- |
| `file_stat` | 统计文件，每行 `次数 日期 域名`，按次数降序 |
| `file_rule` | 规则文件，每行 `full:域名` |
| `gen_rule` | 按 `pattern` 生成的文件 |
| `pattern` | 生成模板，`DOMAIN` 会被替换为域名 |
| `appended_string` | 写在 `gen_rule` 文件第一行 |
| `max_entries` | 累计这么多条新记录就立即写一次文件 |
| `dump_interval` | 写文件间隔（秒），默认 60 |
| `domain_set_url` | 写完后调用的地址，用来通知其它插件重新加载 |
| `enable_flags` | 区分查询中的 AD / CD 标志 |
| `enabled` | 是否启用 |

## 说明

- 只记录，不改变查询。
- 最多记录 262,144 个不同的域名，超出后新域名不再记录，已有域名继续计数。

## 管理接口

| 接口 | 用途 |
| --- | --- |
| `GET /plugins/<tag>/show` | 列出统计 |
| `GET /plugins/<tag>/save` | 立即写文件 |
| `GET /plugins/<tag>/flush` | 清空统计 |

## 与 MosDNS 的差异

学习名单相关的参数 `target_mapper`、`target_mark`、`target_tag` 已移除，改用 [route_learner](/plugins/route_learner)。
