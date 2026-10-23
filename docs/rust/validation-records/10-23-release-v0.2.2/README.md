# 10-23 v0.2.2 发版准备：schema 2 配置包与从 v0.2.1 的在线更新演练

任务 `10-23-rtdns-release-v0.2.2`，基线 rtdns `0c896a79` + 本任务工作树（未提交），file 仓库 `1abaa64` + 工作树（未提交）。
契约：[native-update.md](../../contracts/native-update.md)，发版步骤：[release.md](../../release.md)。不改产品代码；只有版本号、发布元数据、两个发布脚本、文档与演练脚本。
构建与演练在 VM（别名 `mosdns-rust`，Linux x86_64，2 vCPU，systemd 257）的任务专属目录上进行，`-j2`、`CARGO_INCREMENTAL=0`、`CARGO_NET_OFFLINE=true`。
演练在 VM 自建的**只有回环的网络命名空间**里运行（程序、假 GitHub、假上游都在 `127.0.0.1`，没有任何网卡，所以不可能访问公网，也不会与 VM 上原有服务争端口）；只用 RFC 地址
（上游应答 `192.0.2.1`，代理覆盖 `192.0.2.9:7890`）。未登录 `mos`、`rtdns`、`esxi`；未提交、未 push、未建 tag、未发布、未部署。

## R1 配置包与发布元数据

| 项 | 结果 |
| --- | --- |
| 出包 | file 仓库 `1abaa64` 的 `config_rust` 经 `build_config_rust_up.sh`（脚本与 `config_rust` 的副本放在仓库外临时目录，输出也在仓库外；仓库里未跟踪的旧 `config_rust_up.zip` 未动） |
| 包内 manifest | `format 1`、`channel "rust"`、`config_schema 2`、`package_id "rust-config-schema-2"`；18 个托管文件（`config_custom.yaml` + 17 个 `sub_config/*.yaml`），含 `sub_config/route_learner.yaml`，不含 `sub_config/special_groups.yaml`；`create_if_missing` 为 `rule/switch16.txt`=B、`rule/switch17.txt`=A；`delete_files` 为空 |
| 包 sha256 | `df496d9059643ee3d9a270570322e2ed29b60490555985a83587bb25a4cf1913`（19,739 字节，19 个条目；发布要用的就是这一份文件） |
| 元数据 | `scripts/rust-release-metadata.json`：`required_config_schema 2`、`config_package_id "rust-config-schema-2"`、`config.url` `https://github.com/jasonxtt/rtdns/releases/download/v0.2.2/config_rust_up.zip`、`config.sha256` 即上面的值 |
| 版本号 | `rust/native-host/Cargo.toml` 与 `rust/Cargo.lock` 的 `mosdns-native-host` 为 `0.2.2` |
| 清单 | `scripts/build-rust-release.sh v0.2.2`（VM）产出 `rtdns-linux-amd64.tar.gz`（程序自报 `v0.2.2`）与 `rtdns-update-manifest.json`：`format 1`、`channel "rust"`、`required_config_schema 2`、`config_package_id "rust-config-schema-2"`、`config{url,sha256}`、`version "v0.2.2"`、`artifacts{rtdns-linux-amd64.tar.gz{sha256}}` |
| 脚本缺口（已补） | `generate-rust-update-manifest.sh` 原先要 `curl` 下载 `config.url` 来核对包，而包是同一个 Release 的资产，建 Release 之前这个 URL 不存在。新增环境变量 `RUST_CONFIG_PACKAGE=<本地文件>`：核对该文件的 sha256 与包内 manifest（`config_schema`、`package_id`、`channel`）是否等于元数据，而不下载；`build-rust-release.sh` 把环境变量透传（只加注释）。本机验证：对正确文件生成含 `config{url,sha256}` 的清单；对一个 sha256 不符的文件报 `config package hash does not match release metadata` 并退出 1 |
| `docs/rust/release.md` | 新增“带配置包的发版顺序”（出包 → 填元数据 → 提交 → 构建程序与清单 → Release 同时上传程序、清单、配置包 → 建好后核对 URL 的 sha256）与“v0.2.2 升级说明”（学习名单从零开始、旧 `gen/*list.txt`/`*rule.txt` 不再使用可手动删除、requery 不再清空学习名单、用户数据不动）；`native-update.md` 的配置包一条加了“作为同一 Release 的资产”的说明 |
| file 仓库 `config_rust.zip` | 按 `README-rust.md` 的命令（`cd config_rust && zip -r ../config_rust.zip . -x '.DS_Store' -x '*/.DS_Store'`，加 `-FS` 同步删除旧条目）重新生成，只改工作树；解压后与 `config_rust/` 逐文件相同（`diff -r` 无输出），117 个条目；相对旧 zip：少了 16 个已不在 `config_rust/` 的 `gen/` 样例文件，多了 `sub_config/route_learner.yaml`；新 sha256 `ac0406a24e6b729147eb3df27277c518a343ced1b144bc209bf5820b58bd9525` |

## R2 演练（`tools/release-drill/update_drill.py`，复用 `tools/rename-drill/drill.py` 的假 GitHub、假上游与探针）

起点是**真实的 v0.2.1 程序**：`gh release download v0.2.1 -R jasonxtt/rtdns` 下载，程序包 sha256 `0ee9eb99b34efb0bd25816a04b1504daddf008448a3e0ffd43ba5f518ccb9097` 与该 Release 清单一致，在本机下载后拷到 VM。
配置根是 file 仓库 `0412666` 的 `config_rust`（schema 1，`sub_config` 无 `route_learner.yaml`）加 `webinfo/rust_config_update_state.json`（`applied_schema 1`、`required_schema 1`、`package_id rust-config-schema-1`、`binary_version v0.2.1`）。
systemd 单元 `rtdns`：`Restart=always`、`RestartSec=120`，`NetworkNamespacePath` 指向上面的命名空间，监听与生产相同（DNS 53、API 9099）。用户数据：`webinfo/settings.json`（`main_proxy` 覆盖 `192.0.2.9:7890`，五个上游组各一条指向假上游）、
经 API 创建的专属分流组（`special_groups.yaml` 由 v0.2.1 生成，另有 `webinfo/special_upstream_groups.json`）、`rule/whitelist.txt` 追加一行用户规则、`rule/switch10.txt` 置 `A`（包的默认是 `B`）。
假 GitHub 提供 v0.2.2：程序包 + 清单（`config.url` 改指假站，sha256 不变）+ `config_rust_up.zip`。基线：217 个文件做 sha256（不含 `update/`、`cache/`、`gen/`），其中 104 个用户数据文件。

| 场景 | 结果 | 数据 |
| --- | --- | --- |
| 起点 | PASS | health `version v0.2.1`、`applied_config_schema 1`、`required_config_schema 1`；DNS 答 `192.0.2.1` |
| 2 配置包校验失败 | **PASS** | 清单里配置包 sha256 写成全 0：apply → `502 {"error":"配置包校验失败，已取消更新: SHA-256 不一致"}`；程序 sha256、217 个文件、包状态文件、`update/`（空）都不变，health 仍 `v0.2.1`，主进程号不变，`NRestarts 0` |
| 3 新程序不健康 | **PASS** | 候选是自报 `v0.2.2`、`check` 通过、`start` 退出 3 的脚本（经发布脚本打包，配置包有效）：apply 200 → health `update.state rolled_back`、`last_error` “候选进程提前退出（exit status: 3）”；程序 sha256 回到 v0.2.1；**217 个文件逐文件 sha256 与更新前一致（差异：无）**，包状态文件字节相同、`applied_schema 1`；试运行期间用 20 ms 轮询确认文件确实被改过（`config_custom.yaml` 出现 2 个版本、`sub_config/route_learner.yaml` 出现过、包状态 2 个版本）所以回滚是真还原；`route_learner.yaml` 已删除；主进程号不变、`NRestarts 0`、DNS 正常、状态接口 `rollback_performed true` |
| 1 成功路径：更新 | **PASS** | 此前已有一次回滚。检查看到 `v0.2.2`（`config_schema_applied 1`）；apply 200；程序 sha256 等于 v0.2.2 构建；health `version v0.2.2`、`applied_config_schema 2`、`required_config_schema 2`；包状态 `applied_schema 2`、`package_id rust-config-schema-2`、`status success`；`update/installed.json` `v0.2.2`；**主进程号不变、`NRestarts 0`**；配置包只下载 1 次；更新期间 DNS 探针（每 5 ms 一次、200 ms 超时）896 次查询失败 6 次、最长静默 0.622 s（真实生产形态的配置，加载比 rename 演练的最小配置慢；那次是 0.211 s） |
| 1 托管文件 | **PASS** | 18 个托管文件的 sha256 全部等于包 manifest；与更新前不同的文件恰为 `config_custom.yaml`、`sub_config/{domain_output,forward_1,process_v4,process_v6,route_learner,rule_set}.yaml`（route_learner 是新增）和 `webinfo/rust_config_update_state.json`，其余 210 个文件不变 |
| 1 用户数据 | **PASS** | 104 个用户数据文件字节相同：`webinfo/settings.json`、`webinfo/special_upstream_groups.json`、生成的 `sub_config/special_groups.yaml`、`rule/`（含用户规则行与 `switch10.txt` 的 `A`）、`srs/`、`adguard/`；新程序仍列出专属组，有效设置里有 `192.0.2.9:7890` |
| 1 学习 | **PASS** | 对 `learn6.example.com` 发 AAAA，上游空 NOERROR → `/plugins/my_nov6list/show` 列出它（其余四个类别没有）；写盘前 `gen/route_learner.json` 不存在；`systemctl stop` 正常停机后文件写出（`format 1`，`my_nov6list` 含该名字）；再 `start` 后 show 又列出它，DNS 正常 |
| 4 之后再检查 | **PASS** | `update_available false`、`config_schema_applied 2`；无 force 的 apply → `installed:false, restart_required:false`；`{"force":true}` 重装成功：同一进程号、`NRestarts 0`、整个演练里配置包共被下载 3 次（场景 2、3、1 各 1 次），force **没有**再下载；218 个文件（含 `route_learner.yaml`）不变；已学名字仍在 |

## 过程说明

- 演练脚本前几次运行失败都是脚本自身的问题，不是产品问题：`/plugins/*/post` 返回纯文本（`http` 改为文本回退）、`config_before` 引用了尚不存在的 `route_learner.yaml`、状态接口的 `config_schema_required` 是**运行中程序**的要求（v0.2.1 为 1），不是最新发布的。
- **包状态文件的字节**：回滚会用程序自己的序列化重写已存在的状态文件（缩进 JSON，带 `files_updated`）。起初演练把它写成紧凑 JSON，回滚后字段值相同但字节不同，场景 3 的逐文件比对因此失败；
  改为按程序自己的写法播种后字节完全一致。真实生产根里的这个文件由程序/切换工具写出，不受影响；这不是产品缺陷（值未变，`applied_schema` 仍为 1）。
- 候选程序是 VM 上用 `scripts/build-rust-release.sh v0.2.2` 构建的，程序包 sha256 `427139e36cf82788a9eb66e8aa07bafc23947f86dde22b8ee5a538e4d0660683`（程序 `rtdns` 18,854,648 字节，sha256 `8596a936d44e952121242f56970636b69fdb37d56e90576a5969fa072260dc74`）。
  发布时应由规划方在提交后的干净工作树上重新构建，程序包的 sha256 以那次为准（本记录的配置包 sha256 才是要发布的那一份）。
- 本任务未跑 `cargo fmt/clippy/test`（唯一的 Rust 侧改动是 crate 版本号）；程序构建本身在 VM 上成功。
- 把整个工作树（不含 `.git`、`target`、`node_modules`）rsync 到 VM 的任务目录，仅用于构建与演练；结束时已删除。

## 清理

演练脚本结束时停用并删除 `rtdns.service`、`daemon-reload`、删除命名空间与工作目录；VM 上之后无 `rtdns` 单元/进程、无演练命名空间；VM 上原有的 `mosdns` 服务（占用 53/9099 等端口）未被触碰（演练在独立命名空间里，端口不冲突）。
