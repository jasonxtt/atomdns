# fast_mark

给查询打上编号 0–63 的标记位，后续规则按标记分流。标记只在本次查询内有效。

**类型**：匹配器 / 执行器

## 简写（常用）

```yaml
- exec: fast_mark 30            # 打上 30 号标记
- matches: fast_mark 8          # 有 8 号标记
  exec: $forward_cn
- matches: fast_mark 1 5        # 有 1 号或 5 号标记
  exec: reject 3
- matches: "!fast_mark 48"      # 没有 48 号标记
  exec: $unified_matcher
```

## 插件写法

```yaml
- tag: mark_proxy
  type: fast_mark
  args: "12 15"

- tag: main
  type: sequence
  args:
    - matches: $mark_proxy      # 作为条件：有 12 或 15 号标记
      exec: $forward_remote
```

## 标记的来源

| 来源 | 说明 |
| --- | --- |
| `exec: fast_mark N` | 直接打标记 |
| [domain_mapper](/plugins/domain_mapper) | 命中某张表时打该表的 `mark` |
| [route_learner](/plugins/route_learner) | 命中学习结果时打该类别的 `mark` |

## mark

`mark` 是另一套独立的上下文标记，数值范围更大（32 位无符号整数），用法相同：

```yaml
- exec: mark 1000
- matches: mark 1000 1001
  exec: accept
```

## 默认配置中的编号

默认配置已使用 1–17、30、48 等编号，自己添加规则时请避开，32–47 与 49 以后可自由使用。
