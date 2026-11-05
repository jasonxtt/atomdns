# adguard_rule

订阅 AdGuard 格式的广告规则。可以直接用作 `qname` 条件，也可以交给 [domain_mapper](/plugins/domain_mapper) 比对。

**类型**：数据　**可在 WebUI 调整**：是（请求屏蔽 › 广告规则）

## 示例

```yaml
- tag: adguard
  type: adguard_rule
  args:
    dir: adguard
    proxy: $main_proxy

- tag: main
  type: sequence
  args:
    - matches: qname $adguard
      exec: reject 3
```

## 参数

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `dir` | 路径 | 规则目录，必填。订阅清单是其中的 `config.json`，规则文件为 `<id>.rules` |
| `proxy` | 引用 | 下载用的 [socks5_proxy](/plugins/settings-objects) |

## 订阅清单

`<dir>/config.json` 是一个数组：

```json
[
  {
    "id": "a1b2c3",
    "name": "example-list",
    "url": "https://example.com/adblock.txt",
    "enabled": true,
    "auto_update": true,
    "update_interval_hours": 24
  }
]
```

## 规则

- `||example.com^` 屏蔽该域名及子域名；`@@` 开头为例外。
- `/regex/` 为正则规则。
- hosts 写法 `0.0.0.0 example.com`、`127.0.0.1 example.com` 也会被屏蔽。
- 其它格式（SRS、YAML 等）的订阅在下载时自动转换为 AdGuard 文本。

## 说明

- 用作 `qname` 条件时，例外规则（`@@`）生效；交给 `domain_mapper` 时只使用屏蔽规则。
- 默认配置中，「广告屏蔽」开关控制是否启用它。
