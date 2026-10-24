<script setup>
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
import ListEditor from '../../components/rules/ListEditor.vue'
import SpecialGroupLinks from '../../components/rules/SpecialGroupLinks.vue'
import SubscriptionTable from '../../components/rules/SubscriptionTable.vue'
import { ROUTING_SUBSCRIPTIONS } from '../../utils/subscription'
</script>

<template>
  <div class="rt-settings-page">
    <FlagSwitches page="routing" />
    <ListEditor tag="whitelist" title="白名单" hint="名单里的域名优先命中白名单，由国内 DNS 解析。支持 full:、domain:、keyword:、regexp: 等写法。" />
    <ListEditor tag="greylist" title="灰名单" hint="名单里的域名优先命中灰名单，由国外 DNS 解析并返回 FakeIP。写法同白名单。" />
    <ListEditor tag="direct_ip" title="直连 IP" hint="不在任何域名名单里的域名，解析出的 IP 属于这个名单时，该域名归入直连。例如 203.0.113.0/24。" />
    <ListEditor tag="rewrite" title="重定向" hint="每行“域名 IP或域名”，例如 example.com 192.0.2.1，或 test.example example.com。支持 full:、domain: 等写法。" />
    <SubscriptionTable kind="online" title="在线订阅" :catalogs="ROUTING_SUBSCRIPTIONS" hint="中国域名、非中国域名、中国 IP 和两个补充集合，从订阅地址下载并按间隔自动更新。" />
    <SpecialGroupLinks />
  </div>
</template>
