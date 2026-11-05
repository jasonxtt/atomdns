# redirect

把查询改成另一个域名去解析，再把结果以 CNAME 的形式返回给客户端。

**类型**：执行器

## 示例

```yaml
- tag: my_redirect
  type: redirect
  args:
    rules:
      - "old.example.com new.example.com"
    files:
      - "rule/redirect.txt"

- tag: main
  type: sequence
  args:
    - exec: $my_redirect
    - exec: $forward_remote
```

## 参数

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `rules` | 列表 | 直接写在配置里的规则 |
| `files` | 列表 | 规则文件，一行一条；文件不存在会报错 |

## 规则写法

```text
原域名 目标域名
```

原域名默认整域名匹配，也可以写 `domain:` 等前缀。

## 工作方式

`redirect` 只作用于它**后面**的规则：后面的转发、缓存等都按目标域名进行，得到应答后把问题改回原域名，
并加上一条「原域名 → 目标域名」的 CNAME 记录。

需要直接返回 IP 而不是另一个域名时，用 [hosts](/plugins/hosts) 或 [rewrite](/plugins/rewrite)。
