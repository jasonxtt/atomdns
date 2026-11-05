# feature_flag

声明一个可在 WebUI 切换的功能开关，在 `sequence` 里用 `flag` 条件引用。开关会自动显示在所属页面的顶部。

**类型**：设置对象　**可在 WebUI 调整**：是

## 示例

```yaml
- tag: block_aaaa
  type: feature_flag
  args:
    label: IPv6 屏蔽
    description: 屏蔽 AAAA 请求（直接返回空）。
    section: blocking
    kind: bool
    default: false

- tag: core_mode
  type: feature_flag
  args:
    label: 核心运行模式
    description: 兼容模式更快，安全模式防泄露。
    section: mode
    kind: choice
    options:
      - {value: compat, label: 兼容模式}
      - {value: safe, label: 安全模式}
    default: compat
    requery_on_change: true
```

## 参数

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `label` | 是 | 显示名称 |
| `description` | 是 | 说明文字 |
| `section` | 是 | 显示在哪个页面：`mode`、`upstream`、`routing`、`blocking`、`rewrite`、`learning`、`cache`、`client`、`special_groups` |
| `kind` | 是 | `bool`（开关）或 `choice`（多选一） |
| `options` | `choice` 必填 | 至少两项，每项 `value` 与 `label` |
| `default` | 是 | 默认值：`bool` 写 `true` / `false`，`choice` 写选项的 `value` |
| `requery_on_change` | 否 | 值改变后自动清空缓存并预热一次（需要配置中有 [requery](/plugins/requery)） |

`tag` 就是开关的名字：小写字母开头，只含小写字母、数字和下划线，最长 32 个字符。

## 引用

```yaml
- matches: flag block_aaaa                # bool 开关打开
- matches: "!flag block_aaaa"             # bool 开关关闭
- matches: flag core_mode=safe            # choice 开关为 safe
```

## 值的存储

当前值保存在 `webinfo/settings.json` 的 `flags` 段，只记录与 `default` 不同的项。

| 接口 | 用途 |
| --- | --- |
| `GET /api/v1/flags` | 列出全部开关 |
| `POST /api/v1/flags/<名字>` | 设置，`{"value": true}` |
| `DELETE /api/v1/flags/<名字>` | 恢复默认 |

## 与 MosDNS 的差异

取代 MosDNS 的 `switch1`–`switch17`。旧写法会在启动时报错并提示改用 `feature_flag`；旧的 `rule/switchN.txt` 在首次启动时自动迁移一次。
