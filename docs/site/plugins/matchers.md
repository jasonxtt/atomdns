# 匹配器速查

写在 `matches` 里。前加 `!` 取反（需加引号，如 `"!qtype 28"`）；列表中全部满足才算命中。

| 写法 | 命中条件 |
| --- | --- |
| `qname domain:example.com full:a.example` | 查询域名匹配任一规则（不写前缀按 `domain:`） |
| `qname $domestic` | 查询域名在 [domain_set](/plugins/domain_set) 或 [adguard_rule](/plugins/adguard_rule) 中 |
| `qtype 1 28` | 查询类型是其中之一（数字：1 A、28 AAAA、65 HTTPS、12 PTR、6 SOA） |
| `client_ip 192.0.2.0/24 $lan &rule/clients.txt` | 客户端 IP 在其中（CIDR、[ip_set](/plugins/ip_set)、文件） |
| `resp_ip 0.0.0.0/0 $geoip_cn` | 应答中的 A / AAAA 地址在其中 |
| `has_resp` | 已经有应答 |
| `rcode 0 3` | 应答响应码是其中之一（0 NOERROR、2 SERVFAIL、3 NXDOMAIN） |
| `fast_mark 1 5` | 有其中任一 [fast_mark](/plugins/fast_mark) 标记 |
| `mark 1000` | 有其中任一 `mark` 上下文标记 |
| `flag block_aaaa` | [功能开关](/plugins/feature_flag) 打开 |
| `flag core_mode=safe` | 多选开关为指定值 |
| `$tag` | 引用 `fast_mark` 插件 |
| `_true` / `_false` | 总是 / 从不命中 |

## 示例

```yaml
- matches:
    - qtype 28
    - flag block_aaaa
  exec: reject 0

- matches: "!resp_ip $geoip_cn"
  exec: $learn_proxy
```

## 与 MosDNS 的差异

不支持 `qclass`、`cname`、`ptr_ip`、`has_wanted_ans`、`env`、`random` 等匹配器；
`qname` 不支持 `&文件`，请先定义 [domain_set](/plugins/domain_set)。
