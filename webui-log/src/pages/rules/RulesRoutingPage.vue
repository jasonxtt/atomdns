<script setup>
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
import ListEditor from '../../components/rules/ListEditor.vue'
import SpecialGroupLinks from '../../components/rules/SpecialGroupLinks.vue'
import SubscriptionTable from '../../components/rules/SubscriptionTable.vue'
import { ROUTING_SUBSCRIPTIONS } from '../../utils/subscription'

// 分流规则 has many lists, so they sit under third-level tabs (`?tab=<id>`); the switches stay on top.
const TABS = [
  { id: 'subscriptions', label: '在线订阅' },
  { id: 'lists', label: '白/灰名单' },
  { id: 'fakeip', label: 'FakeIP 过滤' },
  { id: 'ip-rewrite', label: '直连 IP 与重定向' },
  { id: 'ddns', label: 'DDNS 域名' }
]
const route = useRoute()
const active = computed(() => TABS.some(tab => tab.id === route.query.tab) ? route.query.tab : TABS[0].id)
</script>

<template>
  <div class="rt-settings-page">
    <FlagSwitches page="routing" />
    <nav class="rt-subtabs" aria-label="分流规则">
      <router-link
        v-for="tab in TABS"
        :key="tab.id"
        :to="{ path: '/rules/routing', query: tab.id === TABS[0].id ? {} : { tab: tab.id } }"
        class="rt-subtab"
        :class="{ active: active === tab.id }"
        replace
      >{{ tab.label }}</router-link>
    </nav>

    <template v-if="active === 'subscriptions'">
      <SubscriptionTable kind="online" title="在线订阅" :catalogs="ROUTING_SUBSCRIPTIONS" hint="中国域名、非中国域名、中国 IP 和两个补充集合，从订阅地址下载并按间隔自动更新。" />
      <SpecialGroupLinks />
    </template>
    <template v-else-if="active === 'lists'">
      <ListEditor tag="whitelist" title="白名单" hint="名单里的域名优先命中白名单，由国内 DNS 解析。支持 full:、domain:、keyword:、regexp: 等写法。" />
      <ListEditor tag="greylist" title="灰名单" hint="名单里的域名优先命中灰名单，由国外 DNS 解析并返回 FakeIP。写法同白名单。" />
    </template>
    <template v-else-if="active === 'fakeip'">
      <ListEditor tag="realiplist" title="!CN fakeip filter" hint="名单里的域名向国外 DNS 解析并返回真实 IP，不使用 FakeIP。适合必须用真实 IP 连接的域名。" />
      <ListEditor tag="cnfakeipfilter" title="CN fakeip filter" hint="名单里的域名向国内 DNS 解析并返回真实 IP，不使用 FakeIP。适合必须用真实 IP 连接的域名。" />
    </template>
    <template v-else-if="active === 'ip-rewrite'">
      <ListEditor tag="direct_ip" title="直连 IP" hint="不在任何域名名单里的域名，解析出的 IP 属于这个名单时，该域名归入直连。例如 203.0.113.0/24。" />
      <ListEditor tag="rewrite" title="重定向" hint="每行“域名 IP或域名”，例如 example.com 192.0.2.1，或 test.example example.com。支持 full:、domain: 等写法。" />
    </template>
    <template v-else>
      <ListEditor tag="ddnslist" title="DDNS 域名" hint="名单里的域名按动态域名处理，适合经常变 IP 的域名；是否优先用国外上游解析，由「运行模式 › DDNS 优先国外解析」开关决定。" />
      <p class="muted rt-list-hint"><router-link to="/rules/mode">前往 运行模式</router-link></p>
    </template>
  </div>
</template>
