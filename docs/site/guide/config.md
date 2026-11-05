# 配置文件结构

AtomDNS 的配置是一个 YAML 文件，结构与 MosDNS v5 相同。启动时用 `-d` 指定配置目录、`-c` 指定主配置文件：

```bash
atomdns start -d /cus/atomdns -c config_custom.yaml
```

## 顶层字段

```yaml
log:
  level: info              # debug / info / warn / error
  file: ""                 # 留空输出到标准错误

api:
  http: "0.0.0.0:9099"     # WebUI 与管理接口

include:
  - sub_config/rules.yaml  # 先加载的子配置

plugins:
  - tag: main
    type: sequence
    args: []
```

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `log` | 是 | 日志级别与输出文件 |
| `api` | 否 | WebUI 与管理接口的监听地址；不写则不提供 WebUI 和接口 |
| `include` | 否 | 子配置文件列表，按顺序加载 |
| `plugins` | 否 | 插件列表 |

其它顶层字段会报错。

## 插件

每个插件有三个字段：

```yaml
- tag: forward_cn          # 名字，全局唯一，其它地方用 $forward_cn 引用
  type: forward            # 插件类型，见「插件」一栏
  args:                    # 参数，因类型而异
    upstreams:
      - addr: 223.5.5.5
```

插件可以按任意顺序定义，引用在后面的插件也没问题。

## include

- 子配置文件里只能有 `plugins`，不能再 `include`。
- 子配置文件的路径相对主配置文件所在目录。
- 主配置与子配置里的插件合在一起，`tag` 不能重复。

## 路径

插件参数里的相对路径（规则文件、缓存文件、状态文件、日志文件等）一律相对**配置目录**（`-d`）解析；
没有给 `-d` 时相对主配置文件所在目录。

## 检查配置

```bash
atomdns check -d /cus/atomdns -c config_custom.yaml
```

只读取和检查，不写任何文件。错误会指出文件、字段与原因，例如：

```text
GAP cache_main cache /cus/atomdns/sub_config/cache.yaml:plugins[0].args.size  expected an integer
```

`check` 还会输出 `EFFECTIVE` 行，列出各项设置的最终值和来源（配置文件或 WebUI 设置）。
