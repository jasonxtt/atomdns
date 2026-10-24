# 10-24 v0.3.0 发版准备：schema 3 配置包、合并后全量验证、从 v0.2.2 的在线更新演练

任务 `10-24-rtdns-release-v0.3.0`，基线 rtdns `72377680` + 本任务工作树（未提交），file 仓库 `725b11e` + 工作树（未提交）。
契约：[native-update.md](../../contracts/native-update.md)、[native-feature-flags.md](../../contracts/native-feature-flags.md)，发版步骤：[release.md](../../release.md)。
不改产品代码；只有版本号、发布元数据、`release.md`、演练脚本与本记录。
构建、测试与演练在 VM（别名 `mosdns-rust`，Linux x86_64，2 vCPU，systemd 257）的任务专属目录上进行，`-j2`、`CARGO_INCREMENTAL=0`、`CARGO_NET_OFFLINE=true`。
演练在 VM 自建的**只有回环的网络命名空间**里运行（程序、假 GitHub、假上游都在 `127.0.0.1`，没有任何网卡，不可能访问公网，也不与 VM 上原有服务争端口）；
只用 RFC 地址（上游应答 `192.0.2.1`，专属组上游应答 `198.51.100.7`，代理覆盖 `192.0.2.9:7890`）。未登录 `mos`、`rtdns`、`esxi`；未提交、未 push、未建 tag、未发布、未部署。

## R0 合并后的全量验证（基线代码，VM）

| 项 | 结果 |
| --- | --- |
| `cargo fmt --all --check` | 通过 |
| `cargo clippy -j2 --workspace --all-targets -- -D warnings` | 通过，无警告 |
| `cargo test -j2 --workspace --no-fail-fast` | **1503 通过 / 0 失败 / 0 忽略**（98 个测试套件） |
| 前端 `node --test webui-log/tests/*.test.mjs` | 76 / 76 通过（先 `npm ci`；未装依赖时有 6 项因缺 `vue` 失败，不是代码问题） |
| 前端 `npm run build` | 通过 |

## R1 配置包与发布元数据

| 项 | 结果 |
| --- | --- |
| 出包 | file 仓库 `725b11e` 的 `build_config_rust_up.sh` 与 `config_rust`（`git archive` 出来放在仓库外临时目录运行，输出也在仓库外；仓库里未跟踪的旧 `config_rust_up.zip` 未动） |
| 包内 manifest | `format 1`、`channel "rust"`、`config_schema 3`、`package_id "rust-config-schema-3"`；18 个托管文件（`config_custom.yaml` + 17 个 `sub_config/*.yaml`），含 `sub_config/flags.yaml`，不含 `sub_config/special_groups.yaml`；`delete_files` 为 `["sub_config/switch.yaml"]`；`create_if_missing` 为空 |
| 包 sha256 | `994e55c9f5707d375066364f6a9ea3522572884959f01c8803da278d68b7ad5d`（20,770 字节，19 个条目；**发布要用的就是这一份文件**，重新出包 sha256 会变） |
| 元数据 | `scripts/rust-release-metadata.json`：`required_config_schema 3`、`config_package_id "rust-config-schema-3"`、`config.url` `https://github.com/jasonxtt/rtdns/releases/download/v0.3.0/config_rust_up.zip`、`config.sha256` 即上面的值 |
| 版本号 | `rust/native-host/Cargo.toml` 与 `rust/Cargo.lock` 的 `mosdns-native-host` 为 `0.3.0` |
| 清单 | `RUST_CONFIG_PACKAGE=… scripts/build-rust-release.sh v0.3.0`（VM）产出 `rtdns-linux-amd64.tar.gz`（程序自报 `v0.3.0`）与 `rtdns-update-manifest.json`：`format 1`、`channel "rust"`、`required_config_schema 3`、`config_package_id "rust-config-schema-3"`、`config{url,sha256}`（同上）、`version "v0.3.0"`、`artifacts{rtdns-linux-amd64.tar.gz{sha256}}` |
| 演练候选 | 程序包 sha256 `bef9e065faa1d46a5a00e947ac4c4af08a9a43b88a1a5014d14724a4d70756cf`（6,530,019 字节）；程序 `rtdns` 17,906,416 字节，sha256 `32a7f0ffb119a4d27770c70e24f7af34ffa6615b7db03aa3e9ac22248dfaa041`。这是**未提交工作树**上的构建；发布时应由规划方在提交后的干净工作树上重新构建，程序包 sha256 以那次为准（配置包 sha256 才是要发布的那一份） |
| `docs/rust/release.md` | "v0.3.0 升级须知（草稿）"与"配置 schema 3"合并定稿为"v0.3.0 升级须知（配置 schema 3）"：配置与开关（迁移时机、试运行不写、旧文件保留）、删除的接口及外置界面会失效、内置 WebUI 变化与浏览器旧设置不迁移、`requeryconfig.json` 要手动补的两行原文、requery 本机 URL 不看端口、可删除的旧文件清单、手动降级步骤 |
| file 仓库 `config_rust.zip` | 按 `README-rust.md` 的命令（加 `-FS` 同步删除旧条目）重新生成，只改工作树；解压后与 `config_rust/` 逐文件相同（`diff -r` 无输出），83 个条目，不再含 `ui/ilog`（旧 zip 117 个条目、其中 32 个 `ilog`）；新 sha256 `686c0a86341d05e21fc48c0f6d91ef1dd24d020c85166093df3db463d28abf28` |

## 演练工具

`tools/release-drill/update_drill_v030.py`（复用 `update_drill.py` 与 `tools/rename-drill/drill.py` 的假 GitHub、假上游、探针、命名空间与清理）。起点是**真实的 v0.2.2 程序**：
`gh release download v0.2.2 -R jasonxtt/rtdns` 在本机下载，程序包 sha256 `e69badf9246492bfb29f0747375b28b2990e957914b9313948a2119b5305bc3f` 与该 Release 清单一致，再拷到 VM。
systemd 单元 `rtdns`：`Restart=always`、`RestartSec=120`，`NetworkNamespacePath` 指向上面的命名空间。
开关迁移的期望值由脚本**独立**按契约迁移表与 `rule/switchN.txt`、包内 `flags.yaml` 的默认值算出，再与程序结果比对。

## R2 演练 A：构造数据

起点：file 仓库 `ccc64c9` 的 schema 2 `config_rust` + 按生产形态（11 个字段、同样顺序）播种的包状态（`applied_schema 2`）。用户数据：`webinfo/settings.json`（`main_proxy` 覆盖 `192.0.2.9:7890`、五个上游组各一条指向假上游）、
经 v0.2.2 API 创建的专属分流组（`listen_port 5301`，名单 `full:group.example.com` 与 `domain:grp.example.com`，自己的上游应答 `198.51.100.7`）、`rule/whitelist.txt` 追加用户规则、
v0.2.2 经 API 设置的 switch3=B、switch12=A、switch16=A、switch6=A、外观文件三个 JSON + `panel_background.bin`、v0.2.2 学到的 3 个 AAAA 名字（`gen/route_learner.json` 经正常停机写出）。
假 GitHub 提供 v0.3.0 候选 + 清单（`config.url` 改指假站，sha256 不变）+ `config_rust_up.zip`。基线 223 个文件做 sha256（不含 `update/`、`cache/`、`gen/`），其中 173 个用户数据文件。

| 场景 | 结果 | 数据 |
| --- | --- | --- |
| 起点 | PASS | health `version v0.2.2`、`applied_config_schema 2`；DNS 答 `192.0.2.1`；专属组名单内名字在主端口答 `198.51.100.7`、端口 5301 上任意名字答 `198.51.100.7`；期望迁移结果为恰好 4 个非默认开关 |
| 2 配置包校验失败 | **PASS** | 清单里配置包 sha256 写成全 0：apply → `502 {"error":"配置包校验失败，已取消更新: SHA-256 不一致"}`；程序、223 个文件、包状态、`settings.json`、`update/`（空）都不变；主进程号不变、`NRestarts 0` |
| 3 新程序不健康 | **PASS** | 候选自报 `v0.3.0`、`check` 通过、`start` 退出 3：apply 200 → `update.state rolled_back`、`last_error` “候选进程提前退出（exit status: 3）”；程序 sha256 回到 v0.2.2；**223 个文件逐文件 sha256 与更新前一致（差异：无）**，包状态字节相同、`applied_schema 2`，`settings.json` 字节相同；试运行期间 20 ms 轮询确认文件确实被改过（`config_custom.yaml` 2 个版本、`flags.yaml` 出现过、`switch.yaml` 消失过、包状态 2 个版本），`settings.json` 始终 1 个版本；回滚后 `flags.yaml` 不在、`switch.yaml` 恢复；v0.2.2 的 17 个 `/plugins/switchN/show` 与升级前逐个相同；主进程号不变、`NRestarts 0`、DNS 正常；学习条目 6 条全在 |
| 1 检查与更新 | **PASS** | 检查看到 `v0.3.0`（`config_schema_applied 2`）；apply 200；程序 sha256 等于 v0.3.0 构建；health `version v0.3.0`、`applied_config_schema 3`、`required_config_schema 3`；包状态 `applied_schema 3`、`package_id rust-config-schema-3`、`status success`；`update/installed.json` `v0.3.0`；**主进程号不变、`NRestarts 0`**；配置包只下载 1 次；更新期间 DNS 探针（每 5 ms 一次、200 ms 超时）1000 次查询失败 4 次、最长静默 0.417 s |
| 1 试运行不写盘 | **PASS** | 事务状态依次出现 `staged → installing → testing → verified`；`installing`/`testing` 期间 `settings.json` 只有 1 个版本（schema 2 的字节），`verified` 之后由最终程序写入迁移结果 |
| 1 托管文件 | **PASS** | 18 个托管文件的 sha256 全部等于包 manifest；`switch.yaml` 已删除、`flags.yaml` 在；与更新前不同的 10 个文件全是托管文件、生成的 `special_groups.yaml`、包状态或 `settings.json` |
| 1 用户数据 | **PASS** | 173 个用户数据文件中 172 个字节相同（`rule/` 含 17 个 `switchN.txt`、`srs/`、`adguard/`、`ui/`、`webinfo/` 含外观文件与 `panel_background.bin`）；唯一不同的是 `webinfo/settings.json` |
| 1 开关迁移 | **PASS** | `settings.json` 除新增 `flags` 外各段与更新前相等；`flags` 恰为 `{core_mode: safe, client_routing: listed_direct, ddns_foreign: true, block_aaaa: true}`，与独立计算相等，代理覆盖 `192.0.2.9:7890` 仍在；`GET /api/v1/flags` 13 项、`migrated true`、每个值等于独立计算，4 个非默认项 `source settings`、其余 `yaml` |
| 1 专属分流组 | **PASS** | 仍列出 1 个组，`special_upstream_groups.json` 相同；生成的 `special_groups.yaml`（1643 字节）用 `flag lazy_cache`、无 `switchN`；名单内新名字在主端口答 `198.51.100.7`，端口 5301 任意名字答 `198.51.100.7`，其它名字仍答 `192.0.2.1` |
| 1 检查与学习 | **PASS** | `rtdns check` 报 `gaps=0 cascades=0 globals=0`；`gen/route_learner.json` 名字与更新前相同，`/plugins/route_learner/status` 200；正常停机后仍有 v0.2.2 学到的全部名字，再启动后 show 又列出 |
| 1 新 WebUI | **PASS** | `GET /` 返回与 v0.2.2 不同的页面（426 字节对 451 字节），引用的 2 个资源都是 200 |
| 1 切换 `dns_mode` | **PASS** | `POST /api/v1/flags/dns_mode {"value":"realip"}` → 200、`requery_triggered true`，requery 状态随即变化（任务已启动） |
| 4 之后再检查 | **PASS** | `update_available false`、`config_schema_applied 3`；无 force 的 apply → `installed:false, restart_required:false`；`{"force":true}` 重装成功：同一进程号、`NRestarts 0`、**没有**再下载配置包（整个演练里共 3 次：校验失败、回滚、成功各 1 次）、223 个文件完全不变、`flags`（含更新后做的 `dns_mode` 改动共 5 项）没有被重新迁移覆盖 |

## R3 演练 B：生产副本

规划方把生产 `/cus/rtdns` 只读复制到 VM `/root/v030-prod-copy/rtdns/`（152 个文件，12 MB）。演练**再复制一份**到演练目录操作，原副本全程只读：演练前后对整棵树做 sha256 汇总，**两次完全相同**。
程序路径与生产同形（演练目录内），监听在同一个只有回环的命名空间里，API 端口取副本的 `webinfo/webui_port_settings.json`（它会替换 `config_custom.yaml` 里 `api.http` 的端口）。
副本里 `settings.json` 的各上游组（并补齐五个常规组）被改成唯一一条指向本机假上游的条目，其余各段保持原样，改完的文件作为基线。**以下只写计数与一致/不一致。**
场景顺序与演练 A 相同（先回滚、再成功更新）：回滚核对的是"回到同一份起始状态"，与先更新再回滚等价。

| 场景 | 结果 | 数据 |
| --- | --- | --- |
| 起点 | PASS | v0.2.2 健康（`applied_config_schema 2`）、DNS 可答、专属分流组接口可用（副本里专属组数为 **0**，所以专属组相关核对在 B 里只是"仍为 0"；组的行为由演练 A 覆盖）、学习状态读入 3968 条（5 个类别），`/plugins/route_learner/status` 200；按迁移表独立算出的非默认开关数为 3；基线 125 个文件做 sha256（不含 `update/`、`cache/`、`gen/`），其中 103 个用户数据文件 |
| 3 新程序不健康 → 回滚 | **PASS** | 125 个文件逐文件 sha256 与更新前一致（差异：无）、程序回到 v0.2.2、包状态 `applied_schema 2`、`settings.json` 字节相同；试运行期间文件确实被改过（`config_custom.yaml` 2 个版本、`flags.yaml` 出现、`switch.yaml` 消失、包状态 2 个版本、`settings.json` 1 个版本）；回滚后 `flags.yaml` 不在、`switch.yaml` 恢复、17 个开关取值与更新前一致、`cache/` 与 `gen/` 共 26 个文件名不变、主进程号不变、`NRestarts 0`、DNS 正常；v0.2.2 的 hand-over 保存会清掉已过期的学习条目（按各类别 ttl，440 条），其余 3528 条全在，另外多 3 条来自演练自己的探针查询 |
| 1 在线更新 | **PASS** | v0.3.0、`applied_config_schema 3`、包状态 `applied_schema 3`；主进程号不变、`NRestarts 0`；配置包只下载 1 次；更新期间 DNS 探针 985 次查询失败 5 次、最长静默 0.622 s；`rtdns check` 报 `gaps=0 cascades=0 globals=0` |
| 1 试运行不写盘 | **PASS** | 状态依次 `staged → installing → testing → verified`；`installing`/`testing` 期间 `settings.json` 只有 1 个版本 |
| 1 托管文件与用户数据 | **PASS** | 18 个托管文件等于包 manifest，`switch.yaml` 已删除；与更新前不同的 9 个文件全是托管文件、生成的 `special_groups.yaml`、包状态或 `settings.json`；103 个用户数据文件中 102 个字节相同（`rule/` 含 17 个 `switchN.txt`、`srs/`、`adguard/`、`ui/`、`webinfo/`），唯一不同的是 `settings.json` |
| 1 开关迁移 | **PASS** | `settings.json` 除新增 `flags` 外各段与更新前相等；`flags` 有 3 项，与按 `rule/switchN.txt` 独立计算的期望**一致**；`GET /api/v1/flags` 13 项、`migrated true`、每个值与独立计算一致，非默认的 3 项来源 `settings`、其余 `yaml` |
| 1 专属组与学习 | **PASS** | 专属组数 0 → 0，`webinfo/special_upstream_groups.json` 相同，`rule/`、`srs/`、`adguard/` 字节相同；生成的 `special_groups.yaml` 无 `switchN` 引用；`gen/route_learner.json` 更新前 3968 条、更新后 3531 条（440 条按 ttl 过期被清掉，所有仍有效 30 分钟以上的条目都在）；`/plugins/route_learner/status`、`/api/v1/special-groups`、`/api/v1/flags` 都正常；正常停机后同样；再启动后 DNS 可答、`/api/v1/flags` 200 |

## 发现与说明

- **没有产品缺陷**；两组演练都一次全过（见"过程说明"里脚本自身的调整）。
- **生产副本的 requery 回调端口**：副本 `requeryconfig.json` 里 URL 的端口与 API 实际监听端口不一致。规划方复核：requery 对主机为回环/未指定地址的 URL 不看端口，一律在程序内部执行（`rust/native-host/src/local_url.rs` 的 `normalized`，与参考实现 `normalizeLocalActionURL` 一致），生产在同样配置下最近一次 requery 正常完成，所以这**不是问题**；`release.md` 已据此说明端口照抄即可。
- **首次迁移写盘会把 `settings.json` 序列化成规范形式**：演练 A 起初播种的上游条目没有 `use_proxy` 字段，迁移写盘后每条多了 `"use_proxy": false`（值等价）。程序自己写出的文件本来就带这个字段（演练 B 的生产副本只多了 `flags`，其它段逐段相等），所以不构成问题；演练脚本改为按程序的规范形式播种。
- 副本里学习条目的过期清理属于契约行为（条目在最后一次确认后 `ttl` 秒过期，每次写盘前清扫），不是丢数据；演练按"仍有效 30 分钟以上的条目必须全在"核对。
- 副本的专属分流组数为 0，所以"所有专属分流组、名单与端口"只在演练 A 里有实质内容（1 个组、名单与端口可用）。

## 过程说明

- 演练脚本前几次运行失败都是脚本自身的问题，不是产品问题：`/plugins/switchN/post` 要显式 `Content-Type: application/json`；事务文件的字段名是 `status` 不是 `state`；`settings.json` 播种形式（见上）；
  迁移写盘后正常停机的学习文件会多出 requery 学到的名字（改成"包含"而非"相等"）；生产副本的 API 端口要读 `webui_port_settings.json`；生产副本的上游要补齐五个常规组才不会走到 YAML 里的真实上游；学习条目要按 ttl 清扫来比较。
- 把整个工作树（不含 `.git`、`target`、`node_modules`、`docs/rust/research`、`PLANNING-PACKAGE-CHANGES.md`）rsync 到 VM 的任务目录，仅用于构建与演练；结束时已删除。
- 主检出里用户另一项工作的未提交内容（`docs/rust/plans/m2-backlog.md` 的改动、`PLANNING-PACKAGE-CHANGES.md`、`docs/rust/research/`）未动、未并入；file 仓库未跟踪的 `config_rust_up.zip` 未动。

## 清理

演练脚本结束时停用并删除 `rtdns.service`、`daemon-reload`、删除命名空间与工作目录；执行者另外删除了 VM 上的任务目录（构建产物、日志、候选、v0.2.2 资产）；
`/root/v030-prod-copy/` 原副本保留（由规划方之后删除）。VM 上原有的 `mosdns` 服务未被触碰。

脚本逐步输出见任务的 `research/validation.md`。
