# sequence

按顺序执行一组规则，是所有分流逻辑的骨架。每条规则由可选的 `matches`（条件）和 `exec`（动作）组成。

**类型**：执行器

## 示例

```yaml
- tag: main
  type: sequence
  args:
    - matches: qtype 65              # 条件
      exec: reject 0                 # 动作
    - exec: $cache_main              # 无条件执行
    - matches:                       # 多个条件，全部满足才执行
        - qname $domestic
        - "!qtype 28"
      exec: $forward_cn
    - matches: has_resp
      exec: accept
    - exec: $forward_remote
```

## 规则

| 字段 | 说明 |
| --- | --- |
| `matches` | 一个条件或条件列表；列表中**全部**满足才执行。省略即无条件 |
| `exec` | 一个动作或动作列表；列表中的动作依次执行 |

条件和动作有两种写法：

- **引用插件**：`$tag`，例如 `exec: $forward_cn`。
- **简写**：`名字 参数`，例如 `qname domain:example.com`、`exec: ttl 300`。完整列表见 [匹配器](/plugins/matchers) 与 [执行器](/plugins/executors)。

条件前加 `!` 表示取反：`"!qtype 28"`。以 `!` 开头的 YAML 字符串要加引号。

## 流程控制

| 动作 | 效果 |
| --- | --- |
| `accept` | 结束当前序列，保留当前应答 |
| `reject [rcode]` | 结束当前序列，返回指定响应码（默认 5 REFUSED，可写 0–15） |
| `return` | 结束当前序列，回到 `jump` 的调用处继续 |
| `jump $seq` | 执行另一个序列，结束后回到这里继续 |
| `goto $seq` | 转到另一个序列执行，不再回来 |
| `exit` | 立即结束整个处理，包括所有上层序列 |
| `try $seq` | 执行另一个序列；其中的 `exit` 只结束该序列，回到这里继续 |

用 `exec: $另一个序列` 调用时，被调用的序列是独立的一层：其中的 `accept`、`reject`、`return` 只结束它自己，调用者继续往下执行；
只有 `exit` 会继续向上结束调用者，直到遇到 `try`。

## 执行顺序要点

- 规则按书写顺序执行，没有命中 `accept`、`reject`、`exit` 等就一直往下走。
- 后面的动作可以覆盖前面的应答，例如先 `forward` 再 `forward`，以最后一次为准。
- `cache` 包住它后面的所有规则：命中缓存时跳过后续规则；未命中时，后续规则得到应答后自动存入缓存。
- 序列走完都没有应答时，客户端收到 REFUSED。

## 与 MosDNS 的差异

- 增加了 `try`。
- 不认识的简写或参数在启动时报错。
