# route_learner / route_learn

记住域名的分流结论，下次查询时直接按结论打标记。概念见 [路由学习](/guide/route-learning)。

`route_learner` 是存储，`route_learn` 是「在什么条件下记住什么」的规则。

**类型**：数据 / 执行器　**可在 WebUI 调整**：是（域名学习）

## 示例

```yaml
- tag: route_learner
  type: route_learner
  args:
    target_mapper: unified_matcher
    categories:
      - tag: my_realiplist
        mark: 11
        label: 记忆直连
        ttl: 604800
      - tag: my_fakeiplist
        mark: 12
        label: 记忆代理
        ttl: 604800

- tag: learn_direct
  type: route_learn
  args:
    learner: route_learner
    learn: my_realiplist
    when: "resp_ip $geoip_cn"
    label: 探测到国内 IP

- tag: main
  type: sequence
  args:
    - exec: $unified_matcher         # 学习结果在这里一并比对
    - matches: fast_mark 11
      exec: $forward_cn
    - exec: $forward_cn
    - exec: $learn_direct            # 应答含国内 IP 时记住
```

## route_learner 参数

| 参数 | 默认 | 说明 |
| --- | --- | --- |
| `target_mapper` | 必填 | 关联的 [domain_mapper](/plugins/domain_mapper)，每个 mapper 只能有一个 |
| `categories` | 必填 | 类别列表，顺序即日志标签的顺序 |
| `file` | `gen/route_learner.json` | 状态文件 |
| `write_interval` | `21600` | 写入间隔（秒），有变化时才写 |
| `max_entries` | `20000` | 每个类别默认容量 |

### 类别

| 参数 | 说明 |
| --- | --- |
| `tag` | 类别名，全局唯一 |
| `mark` | 命中时打的 [fast_mark](/plugins/fast_mark)，1–63 |
| `label` | 日志里显示的标签 |
| `ttl` | 从最后一次确认起保留的秒数，必填 |
| `max_entries` | 覆盖默认容量 |
| `enabled` | 默认 `true`；WebUI 可开关 |

## route_learn 参数

| 参数 | 说明 |
| --- | --- |
| `learner` | `route_learner` 的 tag |
| `learn` | 要记入的类别，一个或列表 |
| `when` | 条件，写法同 `matches`，可以是列表（全部满足） |
| `label` | 原因说明，显示在「域名学习」页 |

## 说明

- 学到的结论从**下一次**查询开始生效，不改变本次查询。
- 只有 `route_learn` 规则再次触发才算确认；仅仅按结论分流不会延长保留时间。
- 类别满了时先删除过期的，再删除最久未确认的一部分。

## 管理接口

| 接口 | 用途 |
| --- | --- |
| `GET /plugins/<类别>/show` | 列出该类别的域名 |
| `GET /plugins/<类别>/flush` | 清空该类别 |
| `GET /plugins/<learner>/entries` | 按条件查询（JSON） |
| `GET /plugins/<learner>/status` | 各类别计数与状态 |
