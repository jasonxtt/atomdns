# rewrite

把命中的域名直接解析为指定的 IP 或另一个域名的地址，并**结束**当前序列。WebUI「分流规则 › 直连 IP 与重定向」编辑的重定向名单就是它。

**类型**：执行器　**可在 WebUI 调整**：是

## 示例

```yaml
- tag: rewrite
  type: rewrite
  args:
    files:
      - "rule/rewrite.txt"
    dns: $sequence_local      # 或写 DNS 地址，如 223.5.5.5

- tag: main
  type: sequence
  args:
    - exec: $rewrite            # 放在最前面
    - exec: $forward_remote
```

## 参数

| 参数 | 说明 |
| --- | --- |
| `files` | 规则文件，至少一个；第一个文件是 WebUI 保存的位置，不存在也可以 |
| `dns` | 目标是域名时用来解析它的 DNS：`host[:port]`（默认端口 53），或 `$序列名`，在进程内用该序列解析，不经过网络、不记入查询日志 |

## 规则写法

```text
# 域名 目标
nas.example.com    192.0.2.10
domain:example.org 2001:db8::1
cdn.example.com    other.example.net
```

- 域名默认整域名匹配，也可以写 `domain:` 等前缀。
- 目标是 IP：只应答同一地址族的 A / AAAA，TTL 5 秒；其它类型返回空应答。
- 目标是域名：用 `dns` 查询目标域名的 A / AAAA，把地址作为原域名的应答返回；查询失败返回 SERVFAIL。

## 与 redirect 的区别

| | `rewrite` | [`redirect`](/plugins/redirect) |
| --- | --- | --- |
| 命中后 | 直接应答并结束 | 继续执行后面的规则 |
| 目标为域名时 | 用 `dns` 解析 | 用后面的规则解析 |
| 应答中的 CNAME | 不加 | 加 |

## 管理接口

| 接口 | 用途 |
| --- | --- |
| `GET /plugins/<tag>/show` | 列出规则 |
| `POST /plugins/<tag>/post` | 替换全部规则，`{"values": ["a.example 192.0.2.1"]}` |
