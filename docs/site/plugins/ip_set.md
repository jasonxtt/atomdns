# ip_set / si_set

IP 表，用于 `client_ip`（按客户端）和 `resp_ip`（按应答地址）条件。`si_set` 是可在线订阅的版本。

**类型**：数据

## ip_set

```yaml
- tag: direct_ip
  type: ip_set
  args:
    ips:
      - "192.0.2.0/24"
      - "2001:db8::/32"
    files:
      - "rule/direct_ip.txt"

- tag: main
  type: sequence
  args:
    - matches: "resp_ip $direct_ip"
      exec: accept
```

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `ips` | 列表 | IP 或 CIDR |
| `files` | 列表 | 文本文件，一行一个 IP 或 CIDR，`#` 开头为注释 |

文件不存在时只警告，按空表处理。

## si_set

从在线订阅加载 IP 表，订阅清单是一个 JSON 文件，WebUI「分流规则 › 在线订阅」编辑的就是它。

```yaml
- tag: geoip_cn
  type: si_set
  args:
    local_config: "srs/geoipcn.json"
    proxy: $main_proxy
```

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `local_config` | 路径 | 订阅清单，必填 |
| `proxy` | 引用 | 下载用的 [socks5_proxy](/plugins/settings-objects) |

订阅清单的写法、支持的格式与 [sd_set_light](/plugins/domain_mapper#sd-set-light-参数) 相同。

## 在条件中引用

```yaml
- matches: "client_ip $lan_clients"                # 引用 ip_set / si_set
- matches: "resp_ip 192.0.2.0/24 &rule/extra.txt"  # 直接写 CIDR，或 & 加文件路径
```

多个来源之间是「或」的关系。`resp_ip` 只看应答 Answer 段里的 A / AAAA 记录。

## 管理接口

`ip_set` 只有一个 `.txt` 文件且没有 `ips` 时可在线编辑：

| 接口 | 用途 |
| --- | --- |
| `GET /plugins/<tag>/show` | 列出 |
| `POST /plugins/<tag>/post` | 替换全部，`{"values": ["192.0.2.0/24"]}` |

## 与 MosDNS 的差异

不支持 `sets`。
