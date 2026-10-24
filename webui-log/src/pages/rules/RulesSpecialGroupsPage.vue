<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { getJSON } from '../../api/http'
import { getRuntimeCapabilities, operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
import SpecialGroupDetail from '../../components/rules/SpecialGroupDetail.vue'
import { settingsBackedProxy } from '../../utils/nativeSettings'
import { setError } from '../../utils/notice'

// The list of special groups; opening one (`?group=<slot>`, or `?group=new`) shows the
// group's whole management in one place.
const route = useRoute()
const router = useRouter()
const groups = ref([])
const sources = ref([])
const capabilities = ref(null)
const globalSocks5 = ref('')
const loading = ref(false)

const selected = computed(() => String(route.query.group || ''))
const opened = computed(() => (selected.value === 'new' ? null : groups.value.find((group) => String(group.slot) === selected.value) || null))
const showDetail = computed(() => selected.value === 'new' || Boolean(opened.value))
const canRead = computed(() => supportsOperation('groups.read'))

function routeLabel(group) {
  if (!group.upstream_active) return '当前不参与分流'
  if (!group.listen_port) return '53 端口生效'
  return group.custom_port_only ? '仅自定义端口生效' : '53 端口和自定义端口都生效'
}

async function load() {
  if (!canRead.value) return
  loading.value = true
  try {
    capabilities.value = await getRuntimeCapabilities()
    const [list, refs, overrides] = await Promise.allSettled([
      getJSON('/api/v1/special-groups'),
      supportsOperation('groups.upstreams') ? getJSON('/api/v1/upstream/sources') : Promise.resolve([]),
      supportsOperation('system.global_overrides') ? getJSON('/api/v1/overrides') : Promise.reject(new Error('unavailable'))
    ])
    if (list.status === 'rejected') throw list.reason
    groups.value = Array.isArray(list.value) ? list.value : []
    sources.value = refs.status === 'fulfilled' && Array.isArray(refs.value) ? refs.value : []
    globalSocks5.value = overrides.status === 'fulfilled' ? String(overrides.value?.socks5 || '').trim() : ''
  } catch (error) {
    setError(`读取专属分流组失败: ${error.message}`)
  } finally {
    loading.value = false
  }
}

function open(slot) {
  router.push({ path: '/rules/special-groups', query: { group: String(slot) } })
}

function close() {
  router.push({ path: '/rules/special-groups' })
}

async function onSaved(saved) {
  await load()
  // A new group opens itself so its list and subscriptions can be set up; the detail of an
  // existing group keeps its instance (and the drafts of its lists) and takes the saved group.
  const slot = Number(saved?.slot) || groups.value.find((group) => group.name === saved?.name)?.slot
  if (selected.value === 'new' && slot) router.replace({ path: '/rules/special-groups', query: { group: String(slot) } })
}

async function onDeleted() {
  await load()
  close()
}

onMounted(() => {
  load()
  window.addEventListener('mosdns-log-refresh', load)
})
onBeforeUnmount(() => window.removeEventListener('mosdns-log-refresh', load))
</script>

<template>
  <div class="rt-settings-page">
    <FlagSwitches page="special-groups" />
    <section class="panel rt-setting-section" data-group-list>
      <div class="rt-list-head">
        <h3 class="rt-setting-section-title">专属分流组</h3>
        <button class="btn primary" type="button" :disabled="!supportsOperation('groups.manage')" data-group-new @click="open('new')">新建专属分流组</button>
      </div>
      <p v-if="!canRead" class="muted rt-setting-note" role="note">{{ operationReason('groups.read') }}</p>
      <template v-else>
        <p class="muted rt-list-hint">专属分流组把一部分域名交给指定的上游，可以有自己的监听端口、手动名单和订阅。</p>
        <p v-if="loading && groups.length === 0" class="muted">读取中…</p>
        <p v-else-if="groups.length === 0" class="muted" data-group-empty>还没有专属分流组。</p>
        <div v-else class="table-wrap adaptive-table-wrap rt-compact-table">
          <table class="rt-group-table">
            <thead><tr><th>名称</th><th>端口</th><th>状态</th><th class="text-right">生效上游</th><th class="rt-col-actions">操作</th></tr></thead>
            <tbody>
              <tr v-for="group in groups" :key="group.slot" :class="{ active: String(group.slot) === selected }" :data-group-row="group.slot">
                <td>{{ group.name }}</td>
                <td>{{ group.listen_port || '-' }}</td>
                <td>{{ routeLabel(group) }}<span v-if="group.upstream_warnings?.length" class="rt-text-warn"> · 有告警</span></td>
                <td class="text-right">{{ group.effective_upstream_count || 0 }}</td>
                <td class="row-actions rt-col-actions"><button class="btn tiny secondary" type="button" @click="open(group.slot)">打开</button></td>
              </tr>
            </tbody>
          </table>
        </div>
      </template>
    </section>
    <SpecialGroupDetail
      v-if="showDetail"
      :key="selected"
      :group="opened"
      :sources="sources"
      :capabilities="capabilities"
      :proxy-switch="settingsBackedProxy(capabilities)"
      :global-socks5="globalSocks5"
      @saved="onSaved"
      @deleted="onDeleted"
      @close="close"
    />
  </div>
</template>
