---
layout: home

hero:
  name: AtomDNS
  text: 家庭网络 DNS 分流器
  tagline: 用 Rust 重写的 MosDNS。沿用原有配置与插件语义，带完整 WebUI，单个静态二进制。
  image:
    src: /logo.png
    alt: AtomDNS
  actions:
    - theme: brand
      text: 快速开始
      link: /guide/quick-start
    - theme: alt
      text: 插件参考
      link: /plugins/forward
    - theme: alt
      text: GitHub
      link: https://github.com/jasonxtt/atomdns

features:
  - icon: 🧭
    title: 精细分流
    details: 国内外分流、FakeIP 与 Redir-Host，专属分流组把指定域名交给专属上游与独立缓存。
  - icon: 🧠
    title: 路由学习
    details: 自动记住直连、代理、无 IPv4 / IPv6 的域名，按 TTL 过期，越用越准。
  - icon: 📡
    title: 在线规则
    details: 订阅文本、SRS、MRS、dat、Clash 规则，保存即下载，下一次查询就生效，无需重启。
  - icon: 🔍
    title: 查询审计
    details: 每条查询的最终分流、实际上游、耗时与应答一目了然，可搜索、排名、下钻。
  - icon: 🛡️
    title: 安全更新
    details: 程序、配置包、WebUI 一起校验后更新，新版本不健康自动回滚。
  - icon: 🦀
    title: 纯 Rust
    details: 单个静态二进制，支持 amd64 / arm64 / armv7，也提供 Docker 镜像。
---

<div class="home-shot">
  <img src="/screenshots/overview.jpg" alt="概览">
  <div class="row">
    <figure><img src="/screenshots/rules.jpg" alt="分流规则"><figcaption>分流规则与在线订阅</figcaption></figure>
    <figure><img src="/screenshots/upstreams.jpg" alt="上游设置"><figcaption>上游分组与延迟</figcaption></figure>
    <figure><img src="/screenshots/logs.jpg" alt="查询日志"><figcaption>查询日志</figcaption></figure>
  </div>
</div>
