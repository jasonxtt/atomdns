# domain_mapper / sd_set_light

`domain_mapper` 用一次查找把域名和多张域名表比对，给命中的查询打上 [fast_mark](/plugins/fast_mark) 标记和日志标签。
`sd_set_light` 是可在线订阅的域名表，通常交给它比对。

**类型**：执行器 / 数据

## 示例

```yaml
- tag: geosite_cn
  type: sd_set_light
  args:
    local_config: "srs/geositecn.json"
    proxy: $main_proxy

- tag: whitelist
  type: domain_set_light
  args:
    files: ["rule/whitelist.txt"]

- tag: unified_matcher
  type: domain_mapper
  args:
    default_mark: 17
    default_tag: "未命中"
    rules:
      - tag: whitelist
        mark: 8
        output_tag: "白名单"
      - tag: geosite_cn
        mark: 16
        output_tag: "中国域名"

- tag: main
  type: sequence
  args:
    - exec: $unified_matcher
    - matches: fast_mark 8
      exec: $forward_cn
```

## domain_mapper 参数

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `rules` | 列表 | 要比对的域名表 |
| `default_mark` | 0–63 | 一张表都没命中时打的标记，`0` 为不打 |
| `default_tag` | 字符串 | 一张表都没命中时的日志标签 |

### rules 项

| 参数 | 说明 |
| --- | --- |
| `tag` | 域名表的 tag：`domain_set_light`、`sd_set_light` 或 `adguard_rule` |
| `mark` | 命中时设置的 fast_mark 位，0–63 |
| `output_tag` | 命中时的日志标签，显示在查询日志的分流信息里 |

一个域名可以同时命中多张表，所有命中表的标记都会打上。走哪条路由由后面 `sequence` 判断标记的**顺序**决定。

## sd_set_light 参数

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `local_config` | 路径 | 订阅清单（JSON），必填 |
| `proxy` | 引用 | 下载规则用的 [socks5_proxy](/plugins/settings-objects) |

订阅清单是一个数组，每项一个规则源，WebUI「分流规则 › 在线订阅」编辑的就是它：

```json
[
  {
    "name": "geosite_cn",
    "type": "geositecn",
    "files": "srs/geosite-cn.srs",
    "url": "https://example.com/geosite/cn.srs",
    "enabled": true,
    "auto_update": true,
    "update_interval_hours": 24
  }
]
```

| 字段 | 说明 |
| --- | --- |
| `files` | 下载后保存的位置 |
| `url` | 下载地址 |
| `enabled` | 是否启用 |
| `auto_update` / `update_interval_hours` | 自动更新与间隔 |
| `format` | 可选：`auto`（默认，自动识别）、`srs`、`text`、`yaml`、`json`、`dat`、`mrs` |
| `category` | `dat` 格式必填，要导入的分类，如 `cn` |
| `enable_regexp` | 是否启用其中的正则规则 |

### 支持的订阅格式

| 格式 | 来源 |
| --- | --- |
| SRS | sing-box 二进制规则集 |
| MRS | mihomo 二进制规则集 |
| dat | v2ray `geosite.dat` / `geoip.dat`（导入一个分类） |
| YAML | Clash / mihomo 规则集（`payload:` 列表） |
| JSON | sing-box 源规则集 |
| 文本 | 每行一条：`domain:`、`DOMAIN-SUFFIX,`、hosts、dnsmasq、`\|\|example.com^` 等常见写法 |

非 SRS 的内容在下载时转换为文本规则保存，无法表达的规则会跳过，并在日志中按原因计数。

## 与 MosDNS 的差异

- `domain_set`、`sd_set` 不能作为 `domain_mapper` 的规则源。
- 下载规则不读取 `HTTP_PROXY` 环境变量，请用 `proxy`。
- 下载失败或内容无法使用时保留旧文件，不会清空规则。
