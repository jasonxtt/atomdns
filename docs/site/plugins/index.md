# 插件一览

插件在 `plugins` 中定义，用 `$tag` 在 [sequence](/plugins/sequence) 里引用。很多功能也有不必单独定义的 [简写](/plugins/executors)。

## 服务器

| 类型 | 用途 |
| --- | --- |
| [`udp_server` / `tcp_server`](/plugins/server) | 监听端口，把查询交给入口序列 |

## 上游与缓存

| 类型 | 用途 |
| --- | --- |
| [`forward` / `aliapi`](/plugins/forward) | 转发到上游 DNS |
| [`cache`](/plugins/cache) | 应答缓存，支持过期保留与落盘 |
| [`fallback`](/plugins/fallback) | 主备上游，主路慢或失败时用备路 |
| [`ecs_handler`](/plugins/ecs_handler) | 为发往上游的查询附加 ECS |
| [`socks5_proxy` / `ecs_address`](/plugins/settings-objects) | 可在 WebUI 修改的代理与 ECS 地址 |

## 数据

| 类型 | 用途 |
| --- | --- |
| [`domain_set` / `domain_set_light`](/plugins/domain_set) | 域名表 |
| [`ip_set` / `si_set`](/plugins/ip_set) | IP 表 |
| [`domain_mapper` / `sd_set_light`](/plugins/domain_mapper) | 一次查找比对多张域名表并打标记 |
| [`adguard_rule`](/plugins/adguard_rule) | AdGuard 广告规则订阅 |
| [`hosts`](/plugins/hosts) | 自定义域名解析 |
| [`redirect`](/plugins/redirect) | 把查询改成另一个域名 |

## 流程与标记

| 类型 | 用途 |
| --- | --- |
| [`sequence`](/plugins/sequence) | 规则序列 |
| [`fast_mark`](/plugins/fast_mark) | 查询上的标记位 |
| [`flow_setter` / `tag_setter`](/plugins/flow_setter) | 设置日志里显示的分流信息 |
| [`feature_flag`](/plugins/feature_flag) | 可在 WebUI 切换的功能开关 |

## AtomDNS 扩展

| 类型 | 用途 |
| --- | --- |
| [`route_learner` / `route_learn`](/plugins/route_learner) | 记住域名的分流结论 |
| [`requery`](/plugins/requery) | 定时重查域名，刷新缓存与学习结果 |
| [`rewrite`](/plugins/rewrite) | 改写解析结果 |
| [`domain_output`](/plugins/domain_output) | 统计查询过的域名 |
| [`webinfo`](/plugins/webinfo) | 为 WebUI 保存 JSON 数据 |

## 不支持的 MosDNS 插件

`arbitrary`、`black_hole`、`query_summary`、`metrics_collector`、`ipset`、`nftset`、`reverse_lookup`、`dual_selector`、
`sd_set`、`quic_server`、`http_server` 等未实现，写在配置里会在启动时报错。
其中一些功能有替代：`dual_selector` 用 `prefer_ipv4` / `prefer_ipv6`，`black_hole` 用 [`hosts`](/plugins/hosts)，
查询统计由 WebUI 提供。
