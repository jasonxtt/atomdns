# webinfo

为 WebUI 保存一份 JSON 数据，例如客户端别名。一般不需要自己使用。

**类型**：数据

## 示例

```yaml
- tag: clientname
  type: webinfo
  args:
    file: webinfo/clientname.json
```

## 参数

| 参数 | 说明 |
| --- | --- |
| `file` | JSON 文件，必填；不存在或为空时视为 `{}` |

## 接口

| 接口 | 用途 |
| --- | --- |
| `GET /plugins/<tag>` | 读取 |
| `PUT /plugins/<tag>` | 整体替换并保存 |

定义了 tag 为 `clientname` 的 `webinfo` 时，WebUI 启用客户端别名功能。
