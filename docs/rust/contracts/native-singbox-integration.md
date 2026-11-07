# AtomDNS 与定制 sing-box 集成：单次策略决策、订阅与节点管理

更新日期：2026-11-07。

本文是当前工作的完整技术记录，覆盖 AtomDNS 的域名级出口决策、私有 EDNS0
协议、sing-box FakeIP 绑定与直接路由、订阅和通用节点管理、配置保存事务、WebUI、
测试证据以及验收边界。除特别标注的测试机快照外，源码路径均指向当前工作区，而不是
已经发布的版本。

> **状态：节点所有权已简化并修复，全部聚焦回归通过，新 AtomDNS 已部署到测试机；
> VLESS 节点通过 API 的添加与删除已验收。部分 UI 交互和异常路径仍待验收，尚未提交或发布。**
>
> - AtomDNS 现在完全接管手动节点：保存时 sing-box 配置里的普通节点全部按 AtomDNS
>   节点列表重建，分组、direct/block 和订阅节点保留。原先区分“上一轮由谁接管”的逻辑
>   已删除（见第 11 节）。
> - `cargo test --workspace` 全部通过，只有一项依赖 macOS 不存在的回环地址的 UDP 测试
>   因环境报错，与本次改动无关（见 17.2）。sing-box 聚焦 Go 测试和 WebUI 110 项测试
>   此前已通过，本轮没有改动。
> - 测试机 AtomDNS 已换成本轮的静态 musl 构建；sing-box 和 WebUI 未变。受控 fixture
>   provider 已导入两个节点。节点选择、测速、重启后的 selector 持久化、错误订阅拒绝和
>   新实例启动失败后的旧实例恢复已有实机证据。
> - 通过 AtomDNS 接口添加 `vless://` 节点后，sing-box 配置和 `proxy` 分组都出现该节点，
>   TLS SNI 与 ws path 保留；删除后两处都移除，`proxy-main`、`proxy-alt` 不受影响，
>   sing-box 一直运行。浏览器中添加并删除临时 SOCKS5H 节点也已完成。
> - 仍未验收：在浏览器里走 VLESS 表单、编辑留空、启停、多 selector 切换、窄屏、完整
>   FakeIP 矩阵，以及 AtomDNS settings 第二步失败时的补偿恢复（见 18.2）。
> - 两个仓库都没有提交；sing-box 还没有 fork 到 `DefaWong` 账号下。没有推送、PR、
>   GitHub Release 或生产部署。

## 1. 目标、范围与非目标

### 1.1 要解决的问题

在主路由 PBR、AtomDNS、sing-box 的组合中，RealIP 流量按主路由策略处理，
FakeIP 流量进入 sing-box。未改造前，AtomDNS 先按域名决定是否返回 FakeIP，
sing-box 收到连接后再从 FakeIP 还原域名并执行自己的 `route.rules`，形成两次决策。
两边规则重复维护，可能为同一个域名选择不同出口。

本方案让 AtomDNS 的 DNS 策略直接携带 sing-box 出口标签。sing-box 在分配 FakeIP
时保存域名和出口，连接回来时直接执行已经确定的出口，不再重新做普通域名路由决策。
同时，AtomDNS WebUI 可以管理 sing-box 原生订阅与手动节点，避免在 AtomDNS 内再实现
一套代理协议或订阅解析器。

### 1.2 已实现的设计范围

- AtomDNS 动态读取 sing-box 的出口、出口组和当前节点。
- 手动灰名单的逐条出口绑定，以及 `cusnocn` 自定义代理订阅的出口绑定。
- 未绑定具体出口的 FakeIP 查询显式清除旧绑定，使用 sing-box 默认路径。
- 私有 EDNS0 Option 在 AtomDNS 到 sing-box 的内部 DNS 查询中传递决策。
- sing-box 保存 `FakeIP <-> domain + outbound`，A/AAAA 共享域名级出口绑定。
- TCP、UDP 和预匹配路径恢复出口；存在显式绑定时跳过普通 `route.rules`。
- 目标出口不存在时严格报错阻断，不回退默认出口。
- sing-box 原生 remote provider、selector、节点选择、延迟测试和订阅更新。
- 手动节点通过一条分享链接导入，由 sing-box 原生 parser 生成完整 outbound JSON。
- 单文件配置的条件保存、原子替换、启动前验证、重载结果查询和失败恢复。
- AtomDNS 与 sing-box 两个持久化资源之间的补偿式保存流程。

### 1.3 非目标与边界

- **不使用 `special_groups` 编码出口。** 它是 AtomDNS 的专属分流组功能，
  不是 sing-box outbound 传输协议；不新增专属上游或监听端口来暗含出口标签。
  `route_group` 也不是本功能的名称。
- **不替代主路由 PBR。** RealIP 流量不进入 sing-box，本方案不会改变其路由。
  只有经 AtomDNS 返回 FakeIP、随后进入 sing-box 的连接使用这套闭环。
- **不实现另一套节点协议栈。** 节点解析、provider 下载、协议配置与健康检查由
  sing-box 负责；AtomDNS 保存管理意图及 sing-box 返回的原生配置。
- **不是上游 sing-box 的通用功能承诺。** 当前实现位于本地定制分支
  `reF1nd-stable-next`，需要与本文所述 AtomDNS 改动配套构建。
- **不是零停机或跨进程 ACID 事务。** 重载期间旧 API 会关闭；补偿恢复不能消除
  进程崩溃、机器断电等情况下两个资源暂时不一致的可能性。
- 不恢复 Go/cgo 混合宿主，不引入 `nft`/`eBPF` 方向，也不改变 AtomDNS 配置包
  schema 或已有 managed-files 边界。

## 2. 组件与运行前提

### 2.1 代码与进程

| 组件 | 当前源码 | 职责 |
|---|---|---|
| AtomDNS | 本仓库 `rust/`、`webui-log/` | DNS 策略、规则出口选择、EDNS0 编码、管理 API 与 WebUI |
| 定制 sing-box | 本地 `~/Projects/golang/sing-box`，分支 `reF1nd-stable-next` | FakeIP、出口存储与直接路由、原生 provider、节点 parser、配置重载 |
| 主路由 PBR | 部署环境，不属于这两个代码改动 | 将 FakeIP 流量送入 sing-box；RealIP 流量保持原路径 |

AtomDNS 是纯 Rust 程序，sing-box 是独立 Go 进程。两者之间只有 DNS 和 HTTP API
协作，没有 cgo、进程内 ABI 或共享内存。

### 2.2 必需的部署条件

1. AtomDNS 的 `$nocnfake` 上游指向定制 sing-box 的 FakeIP DNS 入口。
2. sing-box 的 DNS 入站进入 DNS 模块，不能只设置 `protocol: dns` 就假定劫持生效。
   测试环境使用入站匹配加 `hijack-dns` 路由动作。
3. sing-box 配置启用 FakeIP store；需要跨重启恢复时使用
   `experimental.cache_file`。只有内存 store 时，进程退出后绑定消失。
4. sing-box 以**一个可写配置文件**启动，管理 API 才能提供 `/atomdns/config`。
   多配置目录、合并配置或不可写文件不属于这套保存事务。
5. Clash API 地址和密钥配置在 AtomDNS 的「上游设置 > 代理与 ECS」中。
   密钥不得写入规则、文档、工单或日志示例。
6. 主路由把客户端到 FakeIP 地址段的流量送入 sing-box；不经过 sing-box 的 RealIP
   连接不会读取 AtomDNS 的 outbound 绑定。

### 2.3 协议与配置的兼容边界

- EDNS0 Option `65001` 是本方案自定义的本地协议，不是 IANA 标准。
  两端必须同时升级；不支持它的普通 DNS 服务不会理解该指令。
- `webinfo/settings.json` 增加 `singbox_api` 和 `singbox` 字段，属于运行时管理状态，
  不进入配置包 `managed_files`。
- `srs/cusnocn.json` 的来源条目增加可选 `outbound`，不改变订阅所属类别。
- 灰名单保存行保留 `@outbound` 原文；编译 matcher 时拆出出口，不把 `@` 当域名语法。
- AtomDNS 缓存 dump 的 origin 增加可选 `proxy_outbound`，用于跨缓存发布与恢复保持
  出口身份；缺失该字段按未绑定处理。
- 本功能不改变默认 YAML 的 sequence 名称，也不把普通代理订阅转换为专属分流组。

## 3. 单次策略决策的数据流

```text
客户端 DNS 查询
    |
    v
AtomDNS 规则编译与匹配
    |-- 灰名单：可选 @outbound
    |-- cusnocn 订阅：可选 outbound
    |-- 其他规则：不产生特定 sing-box 出口
    |
    v
RoutingState.proxy_outbound
    |
    +-- 非 FakeIP 路径：按现有 AtomDNS/主路由逻辑处理，不附加本协议
    |
    +-- $nocnfake：先移除客户端同名 Option，再写 SET 或 CLEAR
                         |
                         v
                 sing-box FakeIP DNS
                         |
                         +-- 返回或复用 FakeIP
                         +-- 更新 domain -> outbound
                         +-- FakeIP <-> domain 映射保持原有语义
                         |
                    客户端随后连接 FakeIP
                         |
                         v
                 sing-box 反查 domain + outbound
                         |
                         +-- 有显式出口：直接选择该出口，跳过普通 route.rules
                         |                 不存在则报错并阻断
                         |
                         +-- 无显式出口：保留 sing-box 原有路由/默认出口流程
```

AtomDNS 不替客户端任意 EDNS 指令背书：即使客户端自行发送 code 65001，发往
`$nocnfake` 的内部查询副本也会先剥离它，再使用规则计算结果。其他转发上游不使用这个
出口指令，因而不应把私有 Option 泄露到公网 DNS。

### 3.1 为什么 FakeIP transport 必须收到每次策略查询

sing-box 普通 DNS client cache 若直接返回旧 FakeIP 应答，会绕过 transport，新的 SET
或 CLEAR 就无法更新域名绑定。因此 FakeIP transport 类型的查询禁用这层普通 DNS
缓存。AtomDNS 自己的缓存仍保留，但必须把 `proxy_outbound` 纳入命中条件，见第 6 节。

### 3.2 地址复用也要更新出口

`CreateWithOutbound` 先查找已有域名地址。即使 A 或 AAAA 已分配，收到显式指令时仍调用
`FakeIPStoreOutbound(domain, outbound)`，然后返回原地址。A/AAAA 使用同一个域名键保存
出口，因此同一域名的两种地址族不会出现两个独立的策略绑定。

## 4. 私有 EDNS0 Option 65001 协议

### 4.1 DNS 线格式

Option code 为十进制 `65001`，十六进制 `0xFDE9`。它通过 DNS OPT RR 的
`EDNS0_LOCAL` option 携带，payload 格式如下：

| 偏移 | 字段 | 内容 |
|---|---|---|
| `0` | `version` | `0x01`，当前协议版本 |
| `1` | `op` | `0x01` = SET，`0x02` = CLEAR |
| `2..N` | `outbound` | SET 时为 UTF-8 出口标签；CLEAR 时为空 |

```text
SET proxy-alt:  [01 01 70 72 6f 78 79 2d 61 6c 74]
CLEAR:          [01 02]
```

第一行仅表示字段编码：version 1、op 1、后续字节为 ASCII/UTF-8 标签 `proxy-alt`。
协议没有 revision、长度前缀或额外的域名字段；域名来自 DNS Question，目标地址族来自
A/AAAA QTYPE，Option 长度来自 OPT RR 自身。

### 4.2 AtomDNS 编码规则

`attach_edns_outbound(raw, outbound)`：

1. 读取并保留原始 DNS 查询的 ID、Question 和 Flags 等必要内容。
2. 解析已有 OPT 和其他 EDNS options，移除所有 code 65001。
3. 有具体出口时写入 `[1, 1, ...tag]`，没有时写入 `[1, 2]`。
4. 重建 OPT 和 DNS 长度字段，检查 option 与报文尺寸。
5. 只在 `$nocnfake` 的 exchange invocation 前调用。

节点管理 API 的出口/订阅标签要求非空、最长 255 字节，不含控制字符、空白或 `/`。
灰名单解析器也拒绝空白、控制字符和超过 255 字节的标签，但它不负责验证该标签当前
是否存在于 sing-box；最终缺失出口由 sing-box 严格拒绝。

### 4.3 sing-box 解码规则与容错

`extractFakeIPOutbound` 仅处理 `EDNS0_LOCAL` 且 code 为 65001 的选项：

- 长度小于 2 或 version 不是 1：忽略该 Option，视为无指令。
- SET：去除标签两端空白，返回标签和 `hasDirective=true`。
- CLEAR：返回空标签和 `hasDirective=true`。
- 未知 op 或没有有效 Option：视为无指令。

无指令兼容旧 AtomDNS/普通 FakeIP 客户端：不会仅因缺少 Option 就清除已有绑定。
这也是 CLEAR 必须显式存在的原因。当前 sing-box 解码采用字符串转换和 trim，没有在
该层重复执行完整 UTF-8/标签白名单校验；受信边界是 AtomDNS 管理输入和内部查询构造，
不是任意客户端注入的同名 Option。

## 5. 规则来源、出口选择与优先级

### 5.1 灰名单语法

每行仍然是原有规则，可在末尾添加空白分隔的 `@tag`：

```text
full:service.example @proxy-alt
domain:example.org @proxy-main
domain:default-route.example @default
domain:ordinary-proxy.example
```

- `@proxy-alt`、`@proxy-main` 只是示例标签，不是内置名称。
- `@default` 表示匹配到这条高优先级规则后显式回到默认出口，而不是继承较低优先级
  来源的出口。
- 没有 `@...` 的灰名单规则不主动指定出口。
- 以 `@tag` 开头、缺少域名规则的行会被拒绝并报错，而不是当作空规则保存。
- `GET /plugins/greylist/show` 返回含出口后缀的展示行，`POST /plugins/greylist/post`
  保存后仍保留原始后缀；matcher 使用拆分后的规则部分。
- WebUI 提供出口下拉辅助插入，但最终配置以文本行为准。

### 5.2 `cusnocn` 自定义代理订阅

`SourceEntry`/`SourceView` 的可选 `outbound` 应用于该订阅编译出的规则。空值表示默认
路径。只改变出口、而 URL/文件等下载输入未变且本地规则文件已存在时，不重新下载远程
内容，只重建 matcher 并发布变更。

不同代理订阅若覆盖同一域名，结果还受现有规则编译/匹配顺序影响。WebUI 应把绑定展示
在对应订阅上，不能把单个来源的出口误当成整个 AtomDNS 的全局出口。

### 5.3 层级与同级合并的准确语义

编译时只有实际带有非空出口的规则获得优先级：

| 来源 | `outbound_priority` | 行为 |
|---|---:|---|
| `greylist` | 2 | 可覆盖 `cusnocn` 的出口 |
| `cusnocn` | 1 | 可为代理订阅域名指定出口 |
| 其他列表 | 0 | 不产生特定 sing-box 出口绑定 |

合并时，高优先级替换低优先级；未绑定出口的规则不会清除已合并的父级或其他规则出口。
`@default` 在结果生成时转为 `outbound=None` 但保留非零优先级，因此能显式遮蔽低优先级
绑定，最终发送 CLEAR。

**当前实现没有完整的“同优先级按 `full:`、最长 `domain:`、keyword、regexp 再排序”
保证。** `AccumulatedResult::merge_rule`/`merge_result` 在优先级相等时保留已积累的
出口；实际同级结果依赖 matcher 的匹配和合并顺序。不要为同一域名配置多个互相冲突的
同级 `@outbound` 并假定最长后缀必赢。这一点比早期设计说明更窄，后续若要承诺完整的
规则特异性排序，需要单独修改 matcher 并补充冲突测试。

### 5.4 状态传递

- mapper 确实得到出口决策时才写 `RoutingField::ProxyOutbound`，避免无关 mapper 清除
  先前结果。
- sequence scope 完成、普通 routing 字段清理时保留 `proxy_outbound`，并把它带入
  `ResponseOrigin`。这是修复实际切换 `@proxy-alt` 却被改成默认出口的关键。
- 灰名单、代理订阅或基础规则仍按原有 AtomDNS 语义决定是否进入 FakeIP；outbound
  字段本身不会把 RealIP 域名强制变为 FakeIP。

## 6. 缓存、更新与失效语义

### 6.1 AtomDNS 缓存

缓存查找比较：

```text
hit.origin.proxy_outbound == 当前查询计算出的 state.routing.proxy_outbound
```

不相等就不复用该缓存结果，重新执行 `$nocnfake` 查询。因此新增、切换和移除出口都会
触发新的 SET/CLEAR，而不是在 AtomDNS 缓存 TTL 内一直保留旧策略。持久化 cache dump 的
origin 同样携带可选出口，防止恢复缓存后丢失这层身份。

只比较出口并不替代域名、QTYPE、客户端或其他既有缓存键/发布规则；它是新增的必要
条件，不是完整缓存键的重写。

### 6.2 sing-box 存储

| 存储 | 地址与域名 | 出口绑定 | 生命周期 |
|---|---|---|---|
| `MemoryStorage` | 原有内存映射 | 新增 `outboundCache map[string]string` | 进程内 |
| bbolt cache file | 原有 `fakeip_address`、`fakeip_domain4/6` | 新增 `fakeip_outbound` bucket | 随 cache file 持久化 |

空出口删除 bucket 中的域名记录，非空出口写入或覆盖。异步 flush/retry 队列同时处理
`saveOutbound`，`FakeIPReset` 清理出口队列和 bucket。旧 cache file 没有新 bucket 时按需
创建，原有地址和域名 bucket 结构不变。

**写失败当前记录 warning，仍返回已分配/已有 FakeIP。** 这意味着持久化 I/O 故障不是
同步 DNS 错误；监控不能只检查 DNS 是否有应答，还应检查 sing-box 日志和 cache file
健康状态。跨重启恢复验收尚未完成，不能把内存 map 测试当成实际进程重启证据。

### 6.3 规则删除、出口删除与已有连接

- 删除域名的 `@outbound` 后，下一次实际到达 sing-box 的 FakeIP 查询发送 CLEAR；
  AtomDNS 缓存出口比较用于促使这次查询发生。
- 已建立的连接不会因为 DNS 上的 SET/CLEAR 自动迁移出口。
- selector 的 `interrupt_exist_connections` 仍由 sing-box 配置决定；管理代码在新建
  selector 时默认 false，不强制打开现有连接中断。
- 删除或重命名 outbound 后，旧域名绑定不会自动改写为其他标签。后续连接恢复该标签时
  返回 `outbound not found: <tag>` 并阻断，符合“宁可失败也不偷偷换出口”的要求。
- 清理无用绑定或更换标签需要有意识地更新规则并重查 DNS；不能仅删除节点后期待旧
  FakeIP 记录自动变成默认出口。

## 7. sing-box 直接路由的严格语义

FakeIP 入连接在 `prepareMatchMetadata` 中完成：

1. 判断目标地址是否属于 FakeIP store。
2. `Lookup(address)` 得到域名，把目标恢复为该域名。
3. `LookupOutbound(domain)` 得到非空绑定后写 `InboundContext.FakeIPOutbound`。

有显式 `FakeIPOutbound` 时，普通 `matchRule` 不再遍历 `r.rules`。TCP、UDP 和 PreMatch
分别通过 outbound registry 查找指定标签：

```go
selectedOutbound, loaded = r.outbound.Outbound(metadata.FakeIPOutbound)
if !loaded {
    return E.New("outbound not found: ", metadata.FakeIPOutbound)
}
```

PreMatch 对应拒绝路径，UDP 返回前释放相关缓存包。找到出口后仍经过 sing-box 必要的
公共处理：

- `resolveOutbound`，所以标签可以指向 selector/urltest，由组内当前选择或测速结果落到
  实际节点；
- UDP FakeIP NAT 与目标地址还原；
- 连接 tracker、统计和正常 outbound dial 生命周期。

“跳过普通 route.rules”不等于绕过 sing-box 全部网络栈，也不等于只支持直接 SOCKS5
节点。无显式绑定时仍走原有规则与默认出口。

## 8. AtomDNS 中的 sing-box API 配置与动态发现

运行时设置位于配置根目录下的 `webinfo/settings.json`：

```json
{
  "singbox_api": {
    "addr": "http://127.0.0.1:9090",
    "secret": "<由部署者填写，不写入文档或代码>"
  },
  "singbox": {
    "group": "proxy",
    "providers_managed": true,
    "subscriptions": [],
    "nodes_managed": true,
    "nodes": []
  }
}
```

`singbox.group` 为空时，优先寻找现有配置中标签为 `proxy` 的 selector，否则取第一个
selector，再缺省为 `proxy`。`providers_managed` 和 `nodes_managed` 默认 false，用于区分
“尚未接管这部分配置”和“已接管且列表为空”；空列表具有删除受管对象的意义。

`GET/POST /api/v1/overrides` 读写 `singbox_api`。管理 HTTP client 使用 Bearer Token；
地址需通过 AtomDNS 的 URL 校验。`GET /api/v1/singbox/outbounds` 从 sing-box `/proxies`
读取可选出口，`POST` 用于连接测试。管理页面能力声明为 `singbox.read` 与
`singbox.manage`，后者要求运行时可重载控制存在。

密钥仍属于敏感运行时状态。API 应答、错误转发和排障输出应避免记录密钥、完整认证节点
链接或带认证信息的订阅 URL；本文示例一律使用占位符和文档保留地址。

## 9. 订阅管理

### 9.1 管理模型

每个 `SingboxSubscription` 保存：

| 字段 | 含义与限制 |
|---|---|
| `name` | provider 标签，同时成为订阅节点标签前缀；非空、<=255 字节，无空白或 `/` |
| `url` | `http://` 或 `https://`，必须有 host |
| `update_interval_hours` | 1..8760 小时，生成 sing-box duration，如 `24h` |
| `enabled` | 是否生成到受管 provider 列表及 selector providers |
| `use_proxy` | true 经管理 selector 下载；false 显式直连下载 |

订阅名称创建后不能在编辑接口中直接改名。需要改名时新建订阅、确认可用后删除旧订阅；
这样避免在不知情的情况下改变 `provider/node` 标签和规则绑定。

### 9.2 生成的原生配置

受管订阅生成 `providers` 数组项：

```json
{
  "type": "remote",
  "tag": "example-provider",
  "url": "https://example.invalid/subscription",
  "update_interval": "24h",
  "health_check": { "enabled": false },
  "http_client": { "detour": "proxy" }
}
```

直连时 `http_client` 是显式空对象 `{}`，不是省略字段，也不是 `detour: direct`。
sing-box 定制代码在 `Provider.UnmarshalJSONContext` 单独保留这个空对象的存在性，
避免通用对象合并将其丢成 `nil`；remote provider 在 `IsEmpty()` 时解析空
`HTTPClientOptions`，选择直连。字段省略时才保持 sing-box 默认 HTTP transport 语义。

管理 selector 的 `providers` 只列启用的受管订阅，`use_all_providers=false`，手动节点放在
`outbounds`。provider 节点的名字按 sing-box 原生形式包含 `/`，用于与手动节点区分。

### 9.3 更新与显示

AtomDNS 从 `/providers/proxies` 读取更新时间和节点数，并复用
`PUT /providers/proxies/{name}` 触发下载。WebUI 可新增、编辑、启停、立即更新和删除订阅，
并选择经代理或直连下载。订阅节点只读，不能当成手动节点单独编辑。

provider 支持的实际订阅格式由 sing-box parser 决定；UI 提示的原生、Clash、SIP008 和
分享链接格式不是 AtomDNS 自己实现的兼容保证。受控本地 provider 和错误 URL 有聚焦测试，
但真实外部订阅尚未完成最终实机验收。

## 10. 通用节点链接管理

### 10.1 添加节点，而不是“添加 SOCKS5 节点”

WebUI 的「sing-box > 节点管理」使用一个多行 textarea 接收**一条**节点链接，另有一个
可选标签输入。标签缺省使用链接 `#fragment`。示例只展示形状，不包含真实凭据：

```text
socks5h://user:password@192.0.2.10:1080#example-socks
vless://00000000-0000-0000-0000-000000000000@192.0.2.20:443?security=tls#example-vless
```

当前 sing-box `ParseSubscriptionLink` 支持：

| scheme | 生成 outbound |
|---|---|
| `socks`、`socks5`、`socks5h` | SOCKS version 5，支持 host/port、用户名密码和 fragment tag |
| `vless` | VLESS，保留 parser 支持的 TLS、flow、transport 等选项 |
| `vmess` | VMess |
| `ss` | Shadowsocks |
| `trojan` | Trojan |
| `tuic` | TUIC |
| `hysteria` | Hysteria |
| `hy2`、`hysteria2` | Hysteria2 |
| `anytls` | AnyTLS |

`socks5h` 是分享链接输入别名，生成的 sing-box outbound `type` 是 `socks`，version 是
`5`；不会保留一个名为 `socks5h` 的虚构 outbound 类型。SOCKS parser 要求有效 host 和
1..65535 端口。

### 10.2 解析和原生配置保留

AtomDNS 调用 sing-box：

```text
POST /atomdns/config/parse-node
{"link":"<一条链接>","tag":"<可选覆盖标签>"}
```

sing-box 限制请求体 64 KiB，拒绝空链接和含换行的多条链接。解析后调用 outbound 自己的
`MarshalJSONContext`，返回完整原生 JSON，而非只返回 `type/tag/server`。AtomDNS 的
`SingboxNode` 以 `tag`、`type`、flattened options 和本地 `enabled` 保存，写回 sing-box
时去掉 `enabled` 控制字段，其余协议字段原样合成。

`SingboxNode` 不能使用 `deny_unknown_fields`，否则 VLESS 的 `flow` 等合法协议字段会被
AtomDNS 错拒。`flow`、TLS、WebSocket 等嵌套对象不应经过 AtomDNS 的协议专用表单重建。

编辑时链接框留空表示保留完整旧 outbound，只改变启用状态；不会把已保存的用户名、密码、
UUID 等回显到编辑表单。填入新链接才替换节点配置。节点标签用于规则绑定，当前编辑接口
不允许直接改名，同名 outbound 也不能重复添加。

### 10.3 启停、选择、测速与删除

- 节点列表和出口组来自 sing-box `/proxies`，包含 `all`、`now`、类型、UDP 能力和最近一次
  delay history。
- 选择出口复用 `PUT /proxies/{group}`，不是重写 selector 配置文件。
- 测速复用 `GET /proxies/{name}/delay`，默认测试 URL 为
  `https://cp.cloudflare.com/generate_204`，默认超时 5000 ms，可限制在 100..15000 ms。
- 删除手动节点会从受管 nodes 和 selector 候选中移除；如果原 selector `default` 已失效，
  选择剩余第一个手动节点，没有候选时移除 `default`。
- 如果 DNS 灰名单仍直接绑定被删除节点，sing-box 会按第 7 节严格阻断，而不是使用 selector
  的新 default。UI 的成功提示不代表所有旧规则绑定都已迁移。

## 11. 配置所有权与生成规则

AtomDNS 的 `managed_config(current, management)` 从 sing-box 当前 JSON 克隆结果，只修改
明确接管的 `providers`、`outbounds` 中的管理 selector 和手动节点，保留日志、DNS、inbounds、
实验配置等其余 JSON。

**首次接管。** 在 `nodes_managed` 打开之前，节点列表从现有配置读取：tag 不含 `/`、
类型不是 selector/urltest/loadbalance/direct/block 的 outbound 视为手动节点。provider
节点不会进入手动列表。接管后，AtomDNS settings 是手动节点的唯一来源。

**接管后的规则：AtomDNS 完全拥有手动节点。** 每次保存时：

- 保留 tag 含 `/` 的订阅节点，以及 selector/urltest/loadbalance/direct/block。
- 其余普通 outbound 全部移除，包括不在 AtomDNS 列表里的手工节点和没有 tag 的普通 outbound。
- 再按 AtomDNS 节点列表加入所有启用的节点。停用节点只留在 settings 中，不写入 sing-box。
- selector 的 default 失效时改为第一个剩余手动候选，没有候选时移除 default。

**为什么这样定。** AtomDNS 和 sing-box 都由一键安装脚本从零配置，不需要兼容手工改过的
sing-box 配置。此前尝试按“上一轮由谁接管”区分可删除范围，结果首次接管和后续删除两个
回归用例来回翻转。该逻辑与 `managed_config_with_previous` 已一并删除，`save_managed`
现在只调用 `managed_config`。

**运维含义。** 打开节点管理后，直接写进 sing-box JSON 的手动节点会在下一次通过 AtomDNS
保存时被删除。需要保留的节点必须通过 AtomDNS 添加。

**UI 仍有限制。** 当前 UI 的组选择 watch 只监听 `props.groups`，手动切换到另一个已有
group 时没有同步重置 `selectedNode`；多 selector 场景需要补充交互回归。停用节点不再出现
在 sing-box `/proxies` 的 selector 候选中，“停用后再编辑启用”尚未形成经过验收的完整工作流。
外部同时编辑 sing-box JSON 或跨进程并发写入，仍依靠 revision 冲突与运维恢复兜底。

## 12. sing-box 配置保存、验证、重载与恢复

### 12.1 管理接口

这些接口挂在已有认证 Clash API 下，而不是新的无认证监听：

| 方法与路径 | 作用 |
|---|---|
| `GET /atomdns/config` | 返回配置原文对应的 JSON 和 revision |
| `PUT /atomdns/config` | 条件保存配置并触发重载；最大请求体 16 MiB |
| `GET /atomdns/config/status` | 返回目标 revision、`reloading` 和可选错误 |
| `POST /atomdns/config/parse-node` | 将一条分享链接解析成完整 outbound JSON |

revision 是配置文件**原始内容字节的 SHA-256 十六进制**，不是递增版本号，也不是 AtomDNS
settings 版本。PUT 请求必须带调用方刚刚读取的 revision：

```json
{
  "revision": "<GET 返回的 SHA-256>",
  "config": { "log": {}, "outbounds": [] }
}
```

过期 revision 或已有重载在进行时返回 HTTP 409；配置不可用、解析或验证失败返回相应错误，
不能返回成功后再让坏配置悄悄落盘。

### 12.2 写入前的独立启动验证

`singleFileConfigStore.Save` 在持锁状态下：

1. 检查没有进行中的 reload，并比较当前文件 revision。
2. 在修改正式文件和恢复副本之前验证候选配置。
3. 验证通过一个独立的、无监听 Box 实例完成，至少初始化 outbounds 并启动/下载 providers。
4. 验证实例移除 inbounds、endpoints、services、NTP、network namespaces、普通 route rules
   和 rule-set，避免占用生产入口或依赖无关路由动作。
5. 使用临时目录中的 cache file，避免与生产 `cache.db` 争锁，也避免旧 provider 缓存掩盖
   新 URL 或错误内容；整个验证有 90 秒超时。
6. 验证成功后先把旧文件写到同目录的
   `.<config-name>.atomdns-previous`，再原子替换正式配置。
7. 原子写使用临时文件、0600 权限、sync、close 和 rename，并记录新 revision 为 pending。

这能捕获 provider 下载/启动和 outbound 初始化错误，但不等价于完整生产流量验收。
被刻意移除的 route rules、inbounds 等仍需进程重载和端到端测试覆盖。

### 12.3 重载结果不是 PUT ACK

旧 Box 关闭时会同时关闭当前 Clash API 连接。因此 PUT 成功先写出并 flush：

```json
{"revision":"<new-revision>","reloading":true}
```

然后才触发 `server.router.Reload()`。ACK 只表示候选配置已保存并进入重载，不代表新实例
已健康运行。AtomDNS 随后最多轮询 120 次，每次间隔 250 ms；旧 API 关闭期间的连接错误
和 HTTP 503 会继续等待，直到同 revision 的 `reloading=false`：

- 无 `error`：应用成功。
- 有 `error`：返回失败，sing-box 已尝试恢复旧文件和旧实例。
- 超时：提示人工检查，不把未知状态写成成功。

### 12.4 新实例失败后的恢复

进程 reload loop 保存已知良好的 `previousOptions`。新实例创建失败时：

1. 尝试将 `.atomdns-previous` 恢复到正式配置文件。
2. **无论恢复文件写回是否成功，都使用已知良好的内存 options 重建旧实例**，
   尽量保持管理 API 和网络服务可访问。
3. 通过跨 Box 实例的 `managedReloadResult` 记录失败，供新 API 查询。
4. 若旧实例也无法重建，则进程返回组合错误，不能承诺服务仍可用。

恢复的是“上一次保存前的配置”，不是任意历史版本。手工外部修改、并发写入和机器故障
仍需部署备份与运维恢复。

## 13. AtomDNS 与 sing-box 的双资源保存事务

WebUI 的一次订阅/节点保存同时涉及 sing-box JSON 和 AtomDNS settings。`save_managed`
按以下顺序执行：

1. 从 sing-box 读取当前配置及 revision。
2. 生成候选配置；若与当前配置相同，只应用 AtomDNS settings。
3. 调用 PUT 并等待重载状态，确认 sing-box 已应用候选配置。
4. 成功后应用并保存 AtomDNS settings。
5. 第 4 步失败时重新读取 sing-box revision，并尝试 PUT 回第 1 步的配置。
6. 如果补偿恢复也失败，错误同时说明 AtomDNS 设置失败和 sing-box 恢复失败，要求人工处理。

这样避免把“provider 实际未启动”的订阅先展示成已保存，但不是严格分布式事务。保存中
崩溃、API 不可达、第三方同时修改配置都可能需要依据 revision 和备份核对两边状态。
节点选择与延迟测试是直接调用运行中的 Clash API，不经过这个配置保存事务。

## 14. AtomDNS 管理 API 摘要

以下路径均由 AtomDNS 同源 API 提供；实际可用性受运行时 capability 控制。

| 方法 | 路径 | 作用 |
|---|---|---|
| GET / POST | `/api/v1/overrides` | 读取/保存 `singbox_api` 等运行时覆盖 |
| GET / POST | `/api/v1/singbox/outbounds` | 动态出口列表 / 测试连接 |
| GET / POST | `/api/v1/singbox/subscriptions` | 订阅列表 / 新增订阅 |
| PUT / DELETE | `/api/v1/singbox/subscriptions/{name}` | 编辑或删除订阅；编辑不允许改名 |
| POST | `/api/v1/singbox/subscriptions/{name}/update` | 立即更新启用的订阅 |
| GET / POST | `/api/v1/singbox/nodes` | 出口组和节点列表 / 解析并新增手动节点 |
| PUT / DELETE | `/api/v1/singbox/nodes/{tag}` | 替换/启停或删除手动节点；PUT 空链接保留配置 |
| POST | `/api/v1/singbox/select` | 选择 selector 当前节点，body 为 `group`、`node` |
| POST | `/api/v1/singbox/delay` | 延迟测试，body 为 `name`、可选 `url`、`timeout` |

订阅和节点配置变更需要 runtime control；无法重载时返回 503。下游 sing-box 配置不可用时
返回错误而不是把读取失败显示为空列表。延迟测试的下游错误以该节点的 `error` 字段返回，
便于逐节点展示，不表示测速成功。

## 15. WebUI 工作流

### 15.1 上游设置

「上游设置 > 代理与 ECS」提供 sing-box API 地址、密钥和测试连接。保存后灰名单和
`cusnocn` 订阅编辑器可动态加载出口；下拉数据来自 sing-box，不在 AtomDNS 前端写死节点。

### 15.2 规则设置

- 灰名单编辑器显示 `@outbound` 语法和出口辅助插入。
- `cusnocn` 订阅编辑框提供目标出口，列表显示当前绑定；留空使用默认路径。
- 这些控件改变 DNS 决策，不能理解为 sing-box 节点管理或 `special_groups` 设置。

### 15.3 sing-box 页面

独立 `/sing-box` hash route 分两个子页：

- **订阅管理**：名称、URL、更新间隔、启停、下载方式、节点数、立即更新和删除。
- **节点管理**：选择出口组与当前节点、查看来源和延迟、测速、通过 textarea 添加节点、
  编辑时留空保留配置、启停和删除手动节点。

旧构建中的“添加 SOCKS5 节点”固定表单已经被通用节点链接 UI 替代。2026-11-07 的后续
浏览器记录已在 **1024x768** viewport 中确认：

- 节点页显示“添加节点”、手动两个 SOCKS 节点和 provider 的两个节点；
- 点击添加后出现 `singbox-node-link` textarea、可选 tag、启用开关和“保存并应用”；
- 通过 textarea 添加临时 `acceptance-link` 后，后端配置、settings 和页面节点列表均出现它；
- 订阅页显示“新增订阅”及 fixture 的“更新”“编辑”操作；所查浏览器控制台无错误。

早期 `(empty page)`、`Viewport: 0x0` 是工具/隐藏 pane 的无效读取，不再作为当前页面状态。
上述记录只证明已执行的主流程，不替代临时节点删除、编辑留空、启停、多 selector、窄屏、
错误提示及完整 VLESS 生命周期的浏览器验收。110 项前端测试同样不能代替这些交互检查。

## 16. 源码索引

### 16.1 AtomDNS

| 范围 | 文件 |
|---|---|
| EDNS0 编码与剥离 | `rust/native-host/src/ecs.rs` |
| 规则编译、出口优先级、mapper | `rust/native-host/src/config/providers.rs`、`rust/matcher-core/src/valued.rs` |
| 灰名单/订阅保存与按需重建 | `rust/native-host/src/rule_sources.rs` |
| sequence routing 与响应 origin | `rust/sequence-core/src/state.rs` |
| `$nocnfake` 调用、缓存身份、scope 保留 | `rust/native-host/src/execution.rs` |
| 缓存持久化 | `rust/native-host/src/cache_dump.rs` |
| API/settings 数据结构、校验与旧节点迁移 | `rust/native-host/src/config/settings.rs` |
| 管理 API、配置生成、保存事务、选择/测速 | `rust/native-host/src/singbox_api.rs` |
| 动态出口和 overrides | `rust/native-host/src/settings_api.rs` |
| Bearer HTTP client | `rust/native-host/src/rule_download.rs` |
| API 注册和 capabilities | `rust/native-host/src/api.rs`、`rust/native-host/src/lib.rs` |
| 核心集成测试 | `rust/native-host/tests/m1_f1_typed_settings.rs` |
| API settings/通用节点回归 | `rust/native-host/src/singbox_api.rs`、`rust/native-host/src/config/settings.rs` 的测试模块 |
| WebUI 主页面 | `webui-log/src/pages/SingBoxPage.vue` |
| 订阅与节点组件 | `webui-log/src/components/singbox/` |
| 路由、能力、校验和样式 | `webui-log/src/navigation.js`、`src/pages/index.js`、`src/api/runtimeCapabilities.js`、`src/utils/singbox.js`、`src/style.css` |
| 前端测试 | `webui-log/tests/singbox.test.mjs` 及相关 shell/settings/rules 测试 |

### 16.2 定制 sing-box

| 范围 | 文件（相对 sing-box 仓库根） |
|---|---|
| FakeIP 接口与 inbound metadata | `adapter/fakeip.go`、`adapter/inbound.go` |
| EDNS0 解析与 FakeIP transport | `dns/transport/fakeip/fakeip.go` |
| 地址复用时更新绑定、内存存储 | `dns/transport/fakeip/store.go`、`memory.go` |
| FakeIP transport 绕过普通 DNS cache | `dns/client.go` |
| bbolt bucket 与异步持久化 | `experimental/cachefile/fakeip.go`、`retry.go` |
| 反查、TCP/UDP/PreMatch 直接路由 | `route/route.go` |
| 配置 store/reload tracker 接口 | `adapter/config_store.go` |
| 原子保存、独立验证和恢复 | `cmd/sing-box/managed_config.go` |
| 进程重载与旧实例恢复 | `cmd/sing-box/cmd_run.go` |
| 管理 HTTP API 与节点解析 | `experimental/clashapi/managed_config.go` |
| 分享链接 parser | `provider/parser/link.go` |
| 显式直连 provider 语义 | `option/provider.go`、`provider/remote/remote.go` |
| 核心与路由测试 | `dns/transport/fakeip/store_test.go`、`experimental/cachefile/fakeip_test.go`、`route/route_rule_status_test.go` |
| 配置/API/parser 测试 | `cmd/sing-box/managed_config_test.go`、`experimental/clashapi/managed_config_test.go`、`provider/parser/link_test.go`、`option/provider_test.go` |

## 17. 构建与验证

### 17.1 可复现的聚焦命令

在 AtomDNS 仓库：

```bash
cargo test --manifest-path rust/Cargo.toml -p atomdns-native-host --lib singbox_api::tests
cargo test --manifest-path rust/Cargo.toml -p atomdns-native-host --lib \
  singbox_nodes_read_legacy_socks_and_preserve_generic_protocols
cargo test --manifest-path rust/Cargo.toml -p atomdns-native-host \
  --test m1_f1_typed_settings \
  singbox_api_and_greylist_outbound_forward_edns_65001_and_invalidate_cache \
  -- --exact --nocapture
cargo test --manifest-path rust/Cargo.toml -p atomdns-matcher-core
cargo test --manifest-path rust/Cargo.toml -p atomdns-sequence-core
cd webui-log && npm test && UI_VERSION=v0.6.4 npm run build:package
```

本轮还运行了全量回归：

```bash
cargo test --manifest-path rust/Cargo.toml --workspace
```

它在 macOS 上只有一项环境相关失败，见 17.2。

在 sing-box 仓库：

```bash
go test ./cmd/sing-box ./experimental/clashapi ./provider/parser ./option \
  ./dns/transport/fakeip/... ./experimental/cachefile/... ./route/...
git diff --check
GOOS=linux GOARCH=amd64 CGO_ENABLED=0 \
  go build -trimpath -o /tmp/sing-box-managed-linux-amd64 ./cmd/sing-box
```

正式发布仍须遵守 AtomDNS 的构建/发布门禁；这里产生的 `/tmp` 二进制是测试构建，不是
Release。测试机为 Alpine/OpenRC，使用 `supervise-daemon`，不是 systemd。musl 静态 AtomDNS
构建使用 `x86_64-unknown-linux-musl`，sing-box 使用 `GOOS=linux GOARCH=amd64 CGO_ENABLED=0`。

### 17.2 2026-11-07 实际验证结果

下表合并源码回归和原执行会话留下的实机证据。这里区分“聚焦范围已通过”与“完整
产品验收已完成”，不把 cached test、单次成功请求或新包存在误写成端到端结论。

| 验证 | 最新结果 | 能证明什么 / 不能证明什么 |
|---|---|---|
| sing-box 上述 Go 包 | 全部通过；macOS linker 仅报告重复 `-lobjc` warning | 覆盖 EDNS SET/CLEAR、地址复用更新、bbolt、直接路由、失效出口、配置/API、SOCKS/VLESS parser、显式直连；不是完整生产 E2E |
| sing-box `git diff --check` 与 Linux amd64 构建 | 通过，生成静态 Linux 测试二进制 | 源码可编译；后续已部署到测试机，不是 Release |
| AtomDNS EDNS 集成测试 | 1 passed | API 出口发现、灰名单切换、EDNS 编码与缓存失效的测试环境路径 |
| AtomDNS legacy/generic settings 测试 | 1 passed | 旧简化 SOCKS 设置可读为原生 SOCKS5；通用 VLESS options 可保存 |
| AtomDNS `singbox_api::tests` | **通过** | 简化后的完全接管规则：保留 selector、删除失效 default、通用节点选项往返保留；原 previous-generation 用例随逻辑一并删除 |
| AtomDNS `cargo test --workspace` | **通过，1 项环境失败** | `native-host/tests/client_context.rs` 的 `real_udp_trusted_peers_forged_ecs_and_audit_off` 需要绑定 macOS 上不存在的回环地址，报 `AddrNotAvailable`，与本次改动无关 |
| AtomDNS 编译与灰名单修复 | 通过 | `examples/m1a1_oracle.rs` 补上 `proxy_outbound` 字段后可编译；行首 `@出口` 缺少域名时报错 |
| matcher-core | 各测试目标全部通过 | 当前 matcher 回归通过；不等于已承诺额外同级特异性排序 |
| sequence-core | 各测试目标全部通过 | 当前状态机回归通过 |
| WebUI | **110/110 passed** | 组件静态契约、导航、API operation 和既有 UI 回归；不是浏览器 E2E |
| WebUI production package | 成功；包含 `singbox-node-link`、`socks5h://`、`添加节点` | 新源码可打包；测试机已部署相同节点链接 UI |
| 受控 provider 导入 | fixture 返回 2 个节点，`updated_at` 有值，selector 含 `fixture` provider | 直连下载和原生 provider 路径可用；不是外部公共订阅验收 |
| 节点选择与测速 | 选择 `fixture/fixture-alt` 成功；一次延迟结果为 355 ms | 运行中的 Clash API 选择和测速可用；不证明所有节点都能联网或测速 |
| 重启持久化 | 重启 sing-box 后 selector 仍为 `fixture/fixture-alt`，provider/settings 保留 | selector 与订阅恢复路径通过；随后已恢复 `proxy-main` |
| 错误订阅保存 | HTTP 502，错误包含 provider 404；配置仍只有 fixture，服务保持 started | 启动前验证拒绝坏 provider，不把失败订阅写入正式配置 |
| 真实启动失败恢复 | 使用正确 PUT 方法后返回 200 和目标 revision；随后状态为 `reloading=false` 且含端口冲突错误，配置恢复，服务保持 started，`startup_failure_rollback=passed` | 新实例失败后的错误上报、旧配置恢复、旧实例和 API 可用性均已有实机证据；不覆盖 AtomDNS settings 第二步失败 |
| VLESS 节点 API 生命周期 | `POST /api/v1/singbox/nodes` 返回 200，配置与 `proxy` 分组出现节点，TLS SNI 和 ws path 保留；`DELETE` 返回 200，两处移除，`proxy-main`、`proxy-alt` 不变，sing-box 持续运行 | 新部署 AtomDNS 上的添加/删除和所有权规则成立；WebUI 走同一组接口，但浏览器中的 VLESS 表单、编辑与启停尚未单独验收 |
| 浏览器主流程 | 1024x768 页面确认新节点 textarea，添加临时 SOCKS5H 节点成功；订阅/节点页可读，控制台无错误；临时节点随后删除并在三处状态确认清理 | 新 UI 不再只是源码/包验证；完整交互、VLESS 生命周期与窄屏验收仍未完成 |

实机恢复日志包含：

```text
managed configuration reload failed; restored previous instance:
start service: start inbound: start inbound/mixed[atomdns-proxy-in]:
listen tcp 127.0.0.1:6666: bind: address already in use
```

正确 PUT 验收记录了接受 revision、失败状态中的同 revision、`config_restored=true`、
sing-box 服务 started 和 selector/API 可读，最终输出 `startup_failure_rollback=passed`。
早期 405 是验收脚本方法错误，不是产品恢复失败。当前源码仍有其他已有 deprecated
提示，与本次功能回归结果分开记录。

### 17.3 构建产物与测试机现状

本轮核验的 AtomDNS 与 sing-box 二进制已部署到测试机，不再是“只有本地产物、尚未部署”
的状态。以下是部署核对时的证据，不是发布资产：

| 产物 | 状态 |
|---|---|
| 测试机 `/usr/local/bin/atomdns` | 本轮所有权修复后的静态 musl 构建，`version` 为 `v0.6.4`，SHA-256 前后缀为 `9ec234a2…51b8` |
| 测试机 `/root/atomdns-before-ownership-fix.bak` | 部署前的 AtomDNS 二进制备份，用于回滚 |
| 本机 `/tmp/sing-box-managed-linux-amd64`、测试机 `/usr/local/bin/sing-box` | 静态 Linux amd64 测试构建，SHA-256 均为 `c6e0d0b3f1913f9c0ec77890d68defacfbf02e0085ee94f9e1597e6f478c44d4` |
| 测试机 WebUI | 新节点链接包；`ui_version=v0.6.4`，`built_at=2026-11-07T03:02:39Z`，无旧“添加 SOCKS5 节点”表单，含 `singbox-node-link` |
| 本地后续 WebUI 包 | 后续重复打包时间已到 2026-11-07T03:21:28Z；这是本地产物时间，不能倒写成此前测试机部署时间 |

早期 `/tmp/atomdns-managed-next` 和 `/tmp/atomdns-managed-target/.../atomdns` 曾发生
segmentation fault，不能用于部署。测试机 sing-box 与本地构建 hash 已对上；本轮只替换了
AtomDNS，sing-box 和 WebUI 未变。

测试机 `10.0.0.140` 最近一次只读快照：

- AtomDNS 和 sing-box 均为 started。
- 已配置一个直连 remote provider `fixture`，URL 为测试机本地
  `http://127.0.0.1:18080/sub.json`；API 返回 2 个节点，selector 同时包含手动节点和
  `fixture/fixture-main`、`fixture/fixture-alt`。
- AtomDNS settings 已启用 `providers_managed` 和 `nodes_managed`；验收临时
  `acceptance-link` 已删除，sing-box 配置、settings 和节点 API 均只剩原有手动节点
  `proxy-main`、`proxy-alt`。
- 默认 selector 当前为 `proxy-main`。此前选择 `fixture/fixture-alt`、测速 355 ms 和重启
  持久化均已观察，随后恢复了原选择。
- 新 UI 已部署且浏览器能看到节点 textarea 和 fixture 节点；临时 SOCKS5H 节点的
  添加和删除均已完成，当前干净状态已确认。VLESS 节点已通过 AtomDNS 接口完成添加和删除，
  浏览器表单中的 VLESS 编辑与启停尚待验收。
- 2026-11-06/07 的核心 FakeIP 日志分别观察到 `proxy-main`、`proxy-alt` 被选用；
  2026-11-07 11:04:50 另有 `outbound/socks[fixture/fixture-alt]` 的实际连接日志。
- 11:07:35 的受控端口冲突测试产生“restored previous instance”错误日志，随后配置文件
  恢复且服务继续运行。错误订阅 404 的保存被 HTTP 502 拒绝，未污染正式 providers。

部署前完整恢复点（含旧二进制、配置、settings 和旧 UI）为：

```text
/root/atomdns-singbox-acceptance-20261107-104306/
```

早期恢复文件也仍在：

```text
/root/atomdns-management-backup-20261107-031312/atomdns-settings.json
/root/sing-box-install-v1.14.4/config-before-fixture-subscription.json
```

这些路径只是测试机恢复点，不是通用安装路径；本文不包含 API 密钥或真实节点凭据。

## 18. 已通过范围与尚未完成的验收清单

### 18.1 已有证据支持的范围

- 核心 EDNS/FakeIP、两个手动出口及直接路由聚焦测试通过，曾有实际出口日志。
- 临时 SOCKS5H 节点通过 UI 添加后又删除，清理后的 sing-box 配置、AtomDNS settings
  和节点 API 一致，均只剩两个原手动节点。
- 节点所有权已简化为完全接管，`cargo test --workspace` 除一项环境相关失败外全部通过；
  对应的静态 AtomDNS 已部署到测试机。
- VLESS 节点通过 AtomDNS 接口添加和删除，sing-box 配置、`proxy` 分组和其他手动节点
  的状态符合预期。
- fixture provider 导入两个节点，节点更新和 selector candidates 可读。
- selector 切换、一次成功测速、重启后的选择持久化、错误 provider 拒绝和真实新实例
  启动失败后的旧实例恢复已有实机记录。
- 浏览器主页面、节点链接添加和订阅页面已实际操作，所查控制台无错误。

### 18.2 必须保留的后续事项

1. **完成节点生命周期。** 临时 `acceptance-link` 已删除，sing-box JSON、AtomDNS
   settings 和 selector 候选一致；VLESS 的接口级添加/删除已通过。仍需补编辑留空、替换链接、启停、删除最后一个节点、
   重复 tag、非法链接及被删除节点仍被 DNS 规则引用时的严格阻断。
2. **在浏览器中完成 VLESS 表单验收。** 接口级添加/删除已通过；还需在 WebUI 中添加、
   编辑、启停和删除 VLESS 节点，并确认每步后的后端状态。
3. **完善 UI 交互。** 修复并验证多 selector 切换时的 `selectedNode` 同步、停用节点的
   再启用路径；检查桌面和窄屏、错误提示、加载状态、编辑不回显凭据、订阅启停/删除及
   操作后的真实后端状态。当前只完成了主要桌面流程的一轮验收。
4. **补完整 FakeIP 矩阵。** 对灰名单两个出口、`@default`、删除绑定、A/AAAA、TCP/UDP、
   缓存切换和失效出口阻断分别留下最新 DNS/连接/日志证据；同时核对重启后 bbolt 中
   FakeIP/domain/outbound，而不只核 selector 和 provider。
5. **补保存事务异常路径。** sing-box 端口冲突后的 revision、配置、实例和 API 恢复
   已通过实机验收；另需制造 AtomDNS settings 第二步失败，验证 sing-box 补偿恢复，
   以及补偿本身失败时的可见错误和人工恢复流程。
6. **完成最终干净状态核对。** 移除验收夹具和临时改动或明确保留哪些 fixture，确认
   默认出口、订阅、规则和服务均符合预期，再形成最终验收结论。
7. **最后再评估发布。** 当前两个仓库都没有提交，sing-box 也还没有 fork 到 `DefaWong`
   账号下；没有推送、PR、Release 或生产切换。发布与生产部署
   必须另有明确批准，不能以本文或测试机一轮成功记录替代发布门禁。

## 19. 运维排障要点

- **国外 DNS SERVFAIL 或规则订阅超时**：先检查 `$nocnfake` 是否指向实际运行的 sing-box
  FakeIP 入口、DNS 是否被 `hijack-dns` 送入 DNS 模块，以及 sing-box 默认代理是否可用。
- **修改出口没有生效**：检查 AtomDNS 缓存 origin 是否含新 `proxy_outbound`、请求是否真正
  进入 `$nocnfake`、sing-box FakeIP transport 是否收到 SET/CLEAR、连接是否是旧连接。
- **总是走默认出口**：检查 scope 完成后 `proxy_outbound` 是否保留、mapper 是否被空调用
  覆盖，以及规则是否绑定 selector 名称而非预期具体节点。
- **报 `outbound not found`**：这是显式设计，不是自动 fallback 失效。恢复同名出口或修改
  规则并重查，不要以删除报错来绕过严格策略。
- **provider 显示直连却仍走代理**：确认生成的是 `http_client: {}` 且 sing-box 包含保留空
  对象的补丁；省略字段与显式空对象语义不同。
- **PUT 返回成功后页面报错**：区分“已接受重载”和“重载完成”，读取 `/atomdns/config/status`
  与服务日志；旧 API 短暂断连是预期过程，持续失败则检查恢复结果。
- **独立 sing-box CLI 报 cache file timeout**：检查是否与正在运行的实例争用同一个
  `cache.db`。运行中验收优先使用混合入站和认证 Clash API，不要同时打开生产 cache file。
- **文档和日志安全**：不要贴出 API secret、完整节点 URL、UUID、密码或带 token 的订阅地址；
  调试错误转发也可能包含下游敏感文本，公开前必须脱敏。

---

本文把当前源码行为、已完成的实机验证与未闭环事项分开记录。后续完成节点清理、异常路径
和完整验收，或改变协议与配置所有权时，应同步更新本文状态、测试证据和源码索引；早期
失败记录、单次成功日志或后续本地重新打包都不能替代最新端到端结论。
