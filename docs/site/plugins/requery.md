# requery

定时把最近查询过的域名重新查一遍：先清空指定缓存，再逐个重查，让缓存和 [路由学习](/guide/route-learning) 结果保持最新。

**类型**：后台任务　**可在 WebUI 调整**：是（缓存设置 › 定时刷新）

## 示例

```yaml
- tag: requery
  type: requery
  args:
    file: webinfo/requeryconfig.json
    resolver: $sequence_requery
```

## 参数

| 参数 | 说明 |
| --- | --- |
| `file` | 任务配置与状态文件（JSON），必填；不存在时自动创建 |
| `resolver` | 可选，`$序列名`：在进程内用该序列重查，不占用端口、不记入查询日志；不写时发往任务配置里的 DNS 地址 |

## 任务配置

任务配置由 WebUI 编辑，主要内容：

| 部分 | 说明 |
| --- | --- |
| 域名来源 | 从哪些统计文件读取最近查询过的域名，以及取最近多少天（默认 30 天） |
| 清空列表 | 开始前要清空的缓存（调用其 `flush` 接口） |
| 查询 | 每秒重查多少个域名（默认 100）；没有设置 `resolver` 时还要指定发往的 DNS 地址 |
| 定时 | 是否启用、开始时间、间隔分钟数 |

一次任务的步骤：保存统计 → 读取域名 → 清空缓存 → 逐个查询 A 与 AAAA → 再次保存统计。没有可查询的域名时不清空缓存。

## 说明

- 同一时间只运行一个任务。
- 学习结果不会被清空，它们按各自的保留时间过期，重查会重新确认它们。
- 用 `resolver` 在进程内重查时，刷新流量不会混进查询日志和统计；改用 DNS 地址时，建议给该端口关闭 `enable_audit`。
- 带 `requery_on_change` 的 [功能开关](/plugins/feature_flag) 切换后会自动触发一次。

## 管理接口

| 接口 | 用途 |
| --- | --- |
| `POST /plugins/<tag>/trigger` | 立即运行 |
| `POST /plugins/<tag>/cancel` | 取消运行中的任务 |
| `GET /plugins/<tag>/status` | 查看进度 |
