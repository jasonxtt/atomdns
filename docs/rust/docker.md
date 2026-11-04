# Docker 部署

镜像 `jasonxtt/atomdns`（Docker Hub），平台 `linux/amd64`、`linux/arm64`、`linux/arm/v7`，基于 Debian 12 slim（glibc），内含 glibc 构建的 `atomdns`（amd64；arm64/armv7 在有 `-gnu` 包的版本起也是 glibc，更早的版本里是静态 musl 程序）。glibc 版的内存收缩设置在 musl 上不生效，musl 版常驻内存多十几 MB，所以镜像用 glibc。
标签：`vX.Y.Z`（与程序、GitHub Release 同名）和 `latest`。

## 快速开始

```bash
mkdir -p atomdns-data
docker run -d --name atomdns --restart unless-stopped --network host \
  -e TZ=Asia/Shanghai -v "$PWD/atomdns-data:/cus/atomdns" jasonxtt/atomdns:latest
```

或使用 compose 示例：[host 网络（推荐）](../../docker/docker-compose.host.yml)、[bridge 网络](../../docker/docker-compose.bridge.yml)。
WebUI 在 `http://<主机>:9099/`。

- 配置目录固定为容器内 `/cus/atomdns`，挂载到宿主机目录即可持久化（配置、规则、缓存转储、学习数据、查询日志都在里面）。
- **空目录首次启动**会用镜像内置的全新安装包初始化（含 `ui/main/`），不需要联网。目录里有别的文件却没有 `config_custom.yaml` 时拒绝启动，不会覆盖任何文件。
- 已有配置目录（例如从裸机 `/cus/atomdns` 迁来）直接挂载即可，保留原有数据。
- 默认配置的上游指向宿主机上的伴生服务（`127.0.0.1:6666`、`:1053`）。**host 网络下可直接使用**；bridge 网络下容器内的 `127.0.0.1` 是容器自己，要把这些上游改成宿主机地址（`host.docker.internal` 加 `extra_hosts: host-gateway`）或伴生容器的服务名，否则查询返回 SERVFAIL。
- bridge 网络下，你自己在配置里加的额外监听端口要自己加端口映射（默认配置只有 53 和 API 9099 需要映射）；host 网络没有这个问题，并且查询日志里是真实的客户端地址。
- host 网络下，默认配置只监听 53 和 API 9099，两者都要求宿主机上空闲；任何一个被占用，容器会以 `TCP bind failed: Address in use` 退出重启。端口冲突时改用 bridge 网络（只映射 53 和 9099），或在配置里改掉冲突的 `listen`。
- 容器以 root 运行，监听 53 不需要额外能力。
- 查看版本：`docker run --rm jasonxtt/atomdns version`；检查配置：`docker run --rm -v "$PWD/atomdns-data:/cus/atomdns" jasonxtt/atomdns check -d /cus/atomdns -c config_custom.yaml`。

## 升级

容器**不做在线更新**：WebUI「版本与更新」里的更新功能在所有镜像里都是关闭的（`system.update` 能力报告“容器版请拉取新镜像并重建容器”，`POST /api/v1/update/apply` 返回 501）。升级方式：

```bash
docker pull jasonxtt/atomdns:latest   # 或固定到某个 vX.Y.Z
docker compose up -d                  # 或 docker rm -f atomdns 后重新 docker run
```

镜像里带着在线更新原本会带来的全部东西，新容器启动时（`ATOMDNS_CONTAINER=1`，镜像已设置）会让挂载的配置目录跟上镜像：

1. **配置包**：目录里记录的配置 schema 低于程序要求时，应用镜像内置的 `config_rust_up.zip`，规则与在线更新相同（只替换托管文件 `config_custom.yaml`、`sub_config/*.yaml`，不碰 `webinfo/settings.json` 等用户数据）。替换前的旧文件备份在 `backup/container-schema-<旧>-to-<新>-<时间>/`。
2. **WebUI**：`ui/main/` 是真实目录且 `ui-manifest.json` 的 `ui_version` 与镜像不同时，整体替换为镜像内置的界面。不想要界面的部署删掉 `ui/main/` 即可，镜像不会再创建它。
3. 其它一切（规则、缓存、学习数据、`settings.json`）原样保留。

回退：把镜像换回旧标签即可；配置包只前进不后退，新 schema 的配置不一定能被更旧的程序加载，回退前先看发布说明里该版本是否带了新配置包（`config_schema_required`）。

## 构建与发布（维护者）

镜像由已发布的 GitHub Release 的文件拼装，不在镜像构建里编译：

```bash
scripts/build-docker-image.sh vX.Y.Z           # 构建首个平台到本机 docker，供检查（不推送）
PUSH=1 LATEST=1 scripts/build-docker-image.sh vX.Y.Z   # 推送多平台镜像到 Docker Hub
```

脚本从 Release 下载 `atomdns-linux-{amd64,arm64-gnu,armv7-gnu}.tar.gz`（清单里没有 `-gnu` 包的旧版本用 `arm64`/`armv7` 的 musl 包）、`atomdns-webui.tar.gz`、`config_rust.zip` 和清单，
用清单核对 sha256，按清单的 `config.url` 取 `config_rust_up.zip` 并核对，再用 `docker/Dockerfile` 打包。

**与发版联动：** 镜像跟着 Release 走，不跟着 main 的提交走（在线更新也只认 Release）。在 GitHub 发布正式的 `vX.Y.Z` Release 后，
`.github/workflows/docker-image.yml` 自动构建并推送 `jasonxtt/atomdns:vX.Y.Z` 和 `latest`；草稿、预发布和非 `vX.Y.Z` 的标签不触发。
前提是仓库机密 `DOCKERHUB_USERNAME`、`DOCKERHUB_TOKEN`（Docker Hub 访问令牌）已设置，且 Release 已带齐 `config_rust.zip` 与各程序包
（缺文件时构建会失败，补上后在 Actions 手动运行同一标签即可）。手动运行（`push` 默认关闭）也可用来重建某个标签。

Docker Hub 页面的概览和 compose 示例来自 [docker/README.md](../../docker/README.md)，推送镜像后由同一工作流用 `scripts/sync-dockerhub-readme.sh` 同步；
只改说明时直接在本机运行该脚本即可（用 `DOCKERHUB_USERNAME`/`DOCKERHUB_TOKEN`，未设置时读 `~/.docker/config.json` 的登录）。改 compose 示例要同时改 `docker/docker-compose.*.yml` 与 `docker/README.md`。
推送镜像属于对外发布，先核对再确认。Dockerfile 里没有针对目标平台的 `RUN`，多平台构建不需要模拟器。

环境变量：`ATOMDNS_CONTAINER=1` 打开容器模式（镜像已设）；`ATOMDNS_CONTAINER_ASSETS` 改内置文件目录（默认 `/usr/share/atomdns`，测试用）。
