# 执行器速查

写在 `exec` 里，不用单独定义插件。

## 查询与应答

| 写法 | 作用 |
| --- | --- |
| `forward 223.5.5.5 https://1.1.1.1/dns-query` | 转发到这些上游，见 [forward](/plugins/forward#简写) |
| `cache 4096` | 本处私有的缓存，见 [cache](/plugins/cache#简写) |
| `ttl 300` | 把应答中所有记录的 TTL 改为 300 秒 |
| `ttl 60-3600` | TTL 限制在 60–3600 秒之间 |
| `ecs 203.0.113.0` / `ecs $client_ecs` | 附加 ECS，见 [ecs_handler](/plugins/ecs_handler#简写) |
| `prefer_ipv4` / `prefer_ipv6` | 双栈域名只返回 IPv4 / IPv6 地址；只有一种地址的域名照常返回 |
| `cname_remover` | A / AAAA 应答去掉 CNAME 链，地址记录直接挂在查询域名下 |
| `drop_resp` | 丢弃当前应答 |
| `sleep 500` | 等待 500 毫秒 |

## 标记与日志

| 写法 | 作用 |
| --- | --- |
| `fast_mark 30` | 打 [fast_mark](/plugins/fast_mark) 标记 |
| `mark 1000` | 打 `mark` 上下文标记 |
| `flow_setter group=… sequence=… upstream=…` | 设置日志分流信息，见 [flow_setter](/plugins/flow_setter) |
| `tag_setter 标签` | 设置日志分流标签 |

## 流程控制

| 写法 | 作用 |
| --- | --- |
| `accept` | 结束当前序列，保留应答 |
| `reject 3` | 结束当前序列，返回指定响应码（默认 5） |
| `return` | 回到 `jump` 的调用处 |
| `jump $seq` / `goto $seq` | 执行另一个序列后回来 / 不回来 |
| `try $seq` | 执行另一个序列，其中的 `exit` 只结束该序列 |
| `exit` | 结束整个处理 |
| `$tag` | 执行插件或序列 |

详细语义见 [sequence](/plugins/sequence#流程控制)。

## 与 MosDNS 的差异

不支持 `black_hole`、`query_summary`、`debug_print`、`ipset`、`nftset`、`metrics_collector`、`arbitrary` 等执行器。
