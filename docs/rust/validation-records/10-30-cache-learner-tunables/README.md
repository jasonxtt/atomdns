# 10-30 v0.5.1：缓存与域名学习的参数可在 WebUI 调整（配置 schema 6）

基线 `26609fe8`（v0.5.0 之后）。契约：[native-settings](../../contracts/native-settings.md)、[native-route-learner](../../contracts/native-route-learner.md)、[native-cache-lifecycle](../../contracts/native-cache-lifecycle.md)、[native-update](../../contracts/native-update.md)；升级须知见 [release.md](../../release.md) 的 v0.5.1 一节。

## 改动

| 位置 | 内容 |
| --- | --- |
| `config/settings.rs`、`settings_api.rs`、`api.rs` | 设置层新增 `caches`（`size`/`lazy_cache_ttl`/`dump_interval`）与 `learning`（`write_interval`、各类别 `ttl`/`max_entries`）两段，只存与 YAML 的差异；接口 `/api/v1/cache/settings[/{tag}]`、`/api/v1/learning/settings`；能力 `cache.settings`、`learning.settings`；一次请求只发布一代；缓存身份变化时响应带 `reset` |
| `route_learner.rs` | 交接状态时按新容量保留最近确认的条目 |
| `config.rs`、`config/standard_groups.rs` | 缓存与 `route_learn` 的可选 `label`；专属分流组生成的缓存带标签并使用新默认值，生成后同样经过设置层 |
| `update/mod.rs`、`scripts/rust-release-metadata.json` | `REQUIRED_CONFIG_SCHEMA = 6`、`rust-config-schema-6`；配置包指向 file 仓库提交 `825ccb6e` |
| `webui-log/` | 缓存页的缓存参数（保存前确认）、域名学习页的行内编辑与计数、中文名显示、两个开关改名调序 |
| file 仓库 | `cache.yaml` 新默认值与 `label`、`route_learner.yaml` 的 `label`、`flags.yaml` 开关改名、`build_config_rust_up.sh` 升到 schema 6、`README-rust.md` |

## 验证

| 项 | 结果 |
| --- | --- |
| `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| macOS 全工作区 | 1563 通过；19 个失败与未修改的 main 完全相同（macOS 环境） |
| Linux（构建机）`--lib`(367)、`cache_http`(4)、`f3a_update`(10)、`slice6_management_http`(16)、`route_learner`(13)、`m1_e_standard_groups`(12)、`feature_flags`(16)、`m1_f1_typed_settings`(21) | 全部通过，v0.5.1 版本号下重跑 |
| WebUI `node --test`、`npm run build` | 100 / 100，构建通过 |
| `rtdns check` 完整 `config_rust` | `gaps=0 cascades=0 globals=0` |
| GitHub Actions `release-build.yml`（分支 `feat/cache-learner-tunables`，tag 输入 `v0.5.1`） | 四个程序包、界面包、清单齐全；清单 `required_config_schema 6`、配置包 sha256 与 file 仓库提交里的文件一致（`curl` 取回核对） |

## 在线更新演练（v0.5.0 → v0.5.1，schema 5 → 6）

构建机的回环网络命名空间，本地假 Release 服务器（清单里 `config.url` 改成本地地址，sha256 不变），用 Actions 产出的真实程序包、界面包和配置包；起点是 v0.5.0 的全新安装包 `config_rust.zip`，状态文件置为已应用 schema 5，并先用开关接口写入一个 `settings.json` 覆盖值（`flags.lazy_cache=false`）。

| 步骤 | 结果 |
| --- | --- |
| `update/check`、`update/apply`、`system/restart` | 升级成功：版本 v0.5.1、`required/applied_config_schema` 6/6、`ui/main` 为 v0.5.1、`update.state idle` |
| 新 `cache.yaml` | 7 个缓存 `lazy_cache_ttl 604800`、`dump_interval 21600`，容量 100000×2 / 50000 / 20000×4，7 个中文 `label` |
| 原有 `settings.json` 的开关覆盖 | 保留 |
| `POST /api/v1/cache/settings/cache_cn`（容量 30000、保留 86400） | 200，`reset:["cache_cn"]`；`settings.json` 只多出这两个键 |
| 容量 0、学习写盘间隔 10 秒 | 400，错误信息含字段名和范围，`settings.json` 不变 |
| `POST /api/v1/learning/settings`（写盘间隔、一个类别的有效期与容量） | 200，值写入 `settings.json` |
| 重启 | 缓存与学习的修改值都保持 |
| `DELETE` 恢复默认 | `settings.json` 里 `caches`、`learning` 段消失，只剩原有的 `flags` |

演练后拆除命名空间并删除演练目录。

## 没做

- 没有在真实 mos-test 或生产机器上更新（演练在隔离的命名空间里完成）。
- 容量调小对运行中缓存条数的影响没有在演练里加压测量（集成测试覆盖了淘汰顺序与交接）。
