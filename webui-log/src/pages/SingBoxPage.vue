<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'
import { getJSON } from '../api/http'
import { supportsOperation } from '../api/runtimeCapabilities'
import SingboxNodes from '../components/singbox/NodesPanel.vue'
import SingboxSubscriptions from '../components/singbox/SubscriptionsPanel.vue'

const route = useRoute()
const tabs = [{ id: 'subscriptions', label: '订阅管理' }, { id: 'nodes', label: '节点管理' }]
const active = computed(() => tabs.some((tab) => tab.id === route.query.tab) ? route.query.tab : 'subscriptions')
const status = ref({ loading: true, configured: false, default_outbound: '', outbounds: [], error: '' })
const subscriptions = ref([])
const groups = ref([])
const manualNodes = ref([])

async function load() {
  status.value.loading = true
  const [outbounds, subs, nodes] = await Promise.allSettled([
    getJSON('/api/v1/singbox/outbounds'),
    getJSON('/api/v1/singbox/subscriptions'),
    getJSON('/api/v1/singbox/nodes')
  ])
  if (outbounds.status === 'fulfilled') status.value = { ...outbounds.value, loading: false }
  else status.value = { ...status.value, loading: false, error: outbounds.reason?.message || '读取失败' }
  if (subs.status === 'fulfilled') subscriptions.value = Array.isArray(subs.value?.subscriptions) ? subs.value.subscriptions : []
  if (nodes.status === 'fulfilled') {
    groups.value = Array.isArray(nodes.value?.groups) ? nodes.value.groups : []
    manualNodes.value = Array.isArray(nodes.value?.manual_nodes) ? nodes.value.manual_nodes : []
  }
}

onMounted(() => {
  load()
  window.addEventListener('atomdns-log-refresh', load)
})
onBeforeUnmount(() => window.removeEventListener('atomdns-log-refresh', load))
</script>

<template>
  <div class="rt-settings-page rt-singbox-page">
    <section class="panel rt-setting-section rt-singbox-overview">
      <div class="rt-list-head">
        <div>
          <h2>sing-box</h2>
          <p class="muted rt-list-hint">订阅由 sing-box 原生 provider 下载并维护；AtomDNS 的域名规则可直接绑定下方出口或具体节点。</p>
        </div>
        <router-link class="btn secondary" to="/rules/upstream">API 设置</router-link>
      </div>
      <div v-if="status.error" class="rt-setting-error" role="alert">{{ status.error }}。请先在「上游设置 › 代理与 ECS」保存 sing-box API 地址与密钥。</div>
      <div v-else class="rt-singbox-summary">
        <span>默认出口 <strong>{{ status.default_outbound || '未读取' }}</strong></span>
        <span>{{ status.outbounds?.length || 0 }} 个可选出口</span>
        <span>{{ subscriptions.length }} 条订阅</span>
      </div>
    </section>
    <nav class="rt-subtabs" aria-label="sing-box 管理">
      <router-link v-for="tab in tabs" :key="tab.id" :to="{ path: '/sing-box', query: tab.id === 'subscriptions' ? {} : { tab: tab.id } }" class="rt-subtab" :class="{ active: active === tab.id }" replace>{{ tab.label }}</router-link>
    </nav>
    <SingboxSubscriptions v-if="active === 'subscriptions'" :subscriptions="subscriptions" :groups="groups" @changed="load" />
    <SingboxNodes v-else :groups="groups" :manual-nodes="manualNodes" :outbounds="status.outbounds" @changed="load" />
  </div>
</template>
