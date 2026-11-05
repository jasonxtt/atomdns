# hosts

为指定域名返回自定义地址，类似系统的 hosts 文件。

**类型**：执行器

## 示例

```yaml
- tag: my_hosts
  type: hosts
  args:
    entries:
      - "nas.home 192.0.2.10"
      - "domain:lan.example 192.0.2.20 2001:db8::20"
    files:
      - "rule/hosts.txt"

- tag: main
  type: sequence
  args:
    - exec: $my_hosts
    - matches: has_resp
      exec: accept
    - exec: $forward_remote
```

## 参数

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `entries` | 列表 | 直接写在配置里的规则 |
| `files` | 列表 | 规则文件，一行一条，`#` 开头为注释；文件不存在会报错 |

## 规则写法

```text
域名 地址1 [地址2 ...]
```

- 域名默认整域名匹配（`full:`），也可以写 `domain:`、`keyword:`、`regexp:` 前缀。
- 可以同时写 IPv4 与 IPv6 地址。

## 应答

- 只应答 A / AAAA 查询，TTL 为 10 秒。
- 域名命中但没有对应类型的地址（例如只配置了 IPv4，查询的是 AAAA）时，返回不含地址的空应答（NOERROR 加一条 SOA）。
- hosts 不会结束序列，需要时在后面加 `has_resp` 判断并 `accept`。
