# 固定压测（fixed benchmark）

每次性能改动前后都用这一套量一次，数字才可比。它只测量，不判定正确性；
DNS/路由正确性仍由测试和 `tools/config-diff` 负责。

## 拓扑

| 角色 | 机器 | 内容 |
| --- | --- | --- |
| 负载机 | 4 vCPU Linux（`mosdns-rust`） | dnsperf（CPU 0,1）＋假上游 CoreDNS（CPU 2,3） |
| 被测机 | 2 vCPU / 2 GB VM | 被测 DNS 程序，独占整台 VM，不绑核 |
| 工作站 | Mac | `bench.py` 经 ssh 驱动两边，结果存本地 |

负载机和被测机分开，压测工具不会抢被测程序的 CPU。假上游对任何 A/AAAA 立即回答
文档地址（`198.51.100.7` / `2001:db8::7`），约 10 万 q/s，远高于被测程序所需。

## 文件

- `gen_corpus.py`：固定种子生成查询文件。40% 国内名单域名、30% 国外名单域名、30% 名单外域名，
  85% A / 15% AAAA。`hit.txt` 1 万个固定名字；`miss-N.txt` 每轮 60 万个从不重复的新名字。
- `Corefile.template`、`upstream.sh`：假上游（临时 systemd 单元 `atomdns-bench-upstream`）。
- `prepare_atomdns.py`：复制默认配置包，把所有上游（国内、国外、ECS、FakeIP）改指假上游、
  DNS 监听改到指定端口，其他（规则、缓存、审计容量、学习、开关）保持默认。
- `bench.py`：运行固定阶段并记录结果。

## 每轮阶段（默认 3 轮，同一进程连续跑）

| 阶段 | 内容 | 看什么 |
| --- | --- | --- |
| warm | `hit.txt` 每个名字查一次，5000 q/s | 填缓存 |
| hit_max | `hit.txt` 20 秒，尽可能快 | 缓存命中的极限吞吐 |
| steady_2k | `hit.txt` 30 秒，固定 2000 q/s | 家庭高峰负载下的延迟 p50/p99 |
| miss_max | `miss-<轮>.txt` 15 秒，尽可能快 | 每个都是新域名：匹配规则、问上游、学习、写缓存 |
| idle | 静置 15 秒 | 内存是否回落 |

每个阶段记录：dnsperf 的 QPS、丢失数、延迟直方图（换算 p50/p99/p99.9）；被测进程的 CPU 时间
（所有线程 user+system，换算成「每个查询多少微秒 CPU」，单线程和多线程程序可以公平比较）、
阶段内峰值内存（先清零 VmHWM 再读）和阶段结束时内存。

满载阶段（hit_max、miss_max）的延迟是排队延迟，只说明「已经满了」，不要拿来比较；
延迟看 steady_2k。

## 用法

负载机上（一次性）：dnsperf 2.14 解包到 `~/bench/dnsperf`（Debian 包 `dnsperf`、`libck0t64`、
`libldns3t64`，用 `dpkg-deb -x` 解包，不改系统已装软件），包装脚本 `~/bench/dnsperf.sh` 设置
`LD_LIBRARY_PATH` 后执行它。然后：

```sh
cd ~/bench
kit/upstream.sh start <coredns 程序> <负载机地址> 5300 "2 3"
python3 -I kit/gen_corpus.py --cn-list geosite_cn.txt --out corpus
```

被测机上用 `prepare_atomdns.py` 生成的配置启动 atomdns（例如临时单元
`systemd-run --unit atomdns-bench ... atomdns start -d <root> -c config_custom.yaml`），然后在工作站：

```sh
python3 -I tools/bench/bench.py --load-host <负载机> --sut-host <被测机> \
  --server <被测机地址>:<端口> --process atomdns --label <版本名> --out <本地结果目录>
```

剖析：加 `--perf r1-hit_max,r3-miss_max`（`r<轮>-<阶段>`，逗号分隔），在这些阶段的前 10 秒对被测进程
`perf record -g`，数据留在被测机 `/root/bench/perf/`。调用栈靠帧指针，被测程序要用
`RUSTFLAGS="-C force-frame-pointers=yes" CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` 编译；
VM 里默认用 `cpu-clock` 事件（虚拟机通常没有硬件计数器）。被测机装 `linux-perf`，
想看 glibc 内部函数名再装 `libc6-dbg`。剖析阶段的数字含少量剖析开销，报告里标 `profiled`。

原始输出含内网地址，留在本地结果目录，不提交。整理后的结果写进
`docs/rust/validation-records/`，只用角色名，不写地址。
