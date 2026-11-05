# domain_set / domain_set_light

域名表。`domain_set` 用于 `qname` 条件；`domain_set_light` 是交给 [domain_mapper](/plugins/domain_mapper) 统一比对的轻量版本。

**类型**：数据　**可在 WebUI 调整**：`domain_set_light` 的名单可在「分流规则」「请求屏蔽」中编辑

## 示例

```yaml
- tag: domestic
  type: domain_set
  args:
    exps:
      - "domain:example.cn"
    files:
      - "rule/domestic.txt"

- tag: main
  type: sequence
  args:
    - matches: qname $domestic
      exec: $forward_cn
```

## 参数

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `exps` | 列表 | 直接写在配置里的规则 |
| `files` | 列表 | 规则文件，相对配置目录；一行一条，`#` 开头为注释 |

`domain_set_light` 同样接受 `exps` 与 `files`，文件也可以是 SRS 格式。

## 规则写法

| 写法 | 匹配 |
| --- | --- |
| `domain:example.com` | `example.com` 及其所有子域名 |
| `full:example.com` | 只匹配 `example.com` |
| `keyword:example` | 域名中包含 `example` |
| `regexp:^ad\.` | 正则表达式 |
| `example.com` | 不写前缀，等同 `domain:` |

## 两者的区别

| | `domain_set` | `domain_set_light` |
| --- | --- | --- |
| 在 `qname $tag` 中使用 | 可以 | 不可以 |
| 交给 `domain_mapper` | 不可以 | 可以 |
| SRS 文件 | 不支持 | 支持 |

默认配置里的白名单、灰名单、黑名单等都是 `domain_set_light`，由一个 `domain_mapper` 一次比对完。

## 管理接口

规则来自单个 `.txt` 文件时可在线编辑，保存后对下一次查询生效：

| 接口 | 用途 |
| --- | --- |
| `GET /plugins/<tag>/show` | 列出规则 |
| `POST /plugins/<tag>/post` | 替换全部规则，`{"values": ["domain:a.com", "full:b.com"]}` |

## 与 MosDNS 的差异

- 不支持 `sets`（引用其它域名表）。
- `full:` 规则不区分大小写。
- 规则中不能运行的正则表达式会在启动时报错并指出规则，不会被静默丢弃。
