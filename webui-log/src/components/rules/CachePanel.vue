<script setup>
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { capabilityFetch, operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { getJSON, getText } from '../../api/http'
import { loadCacheInventory, parseCacheMetrics, summarizeCacheFlush } from '../../utils/cacheInventory'
import { openConfirm } from '../../utils/confirm'
import { setError, setSuccess } from '../../utils/notice'
import CacheEntriesModal from './CacheEntriesModal.vue'

// Cache statistics (from /metrics), clearing one cache or all, and looking into a cache.
const rows = ref([])
const loadError = ref('')
const loading = ref(false)
const clearingAll = ref(false)
const clearing = reactive({})
const viewing = ref(null)

const number = (value) => (value === null || value === undefined ? '—' : Number(value).toLocaleString())

function buildRows(caches, metrics) {
  rows.value = caches.map((cache) => {
    const stats = parseCacheMetrics(metrics, cache.tag)
    const rate = (part) => (stats.query_total === null || part === null ? '—' : `${stats.query_total > 0 ? ((part / stats.query_total) * 100).toFixed(2) : '0.00'}%`)
    return { ...cache, ...stats, hit_rate: rate(stats.hit_total), lazy_hit_rate: rate(stats.lazy_hit_total) }
  })
}

async function load() {
  if (!supportsOperation('cache.inventory')) return
  loading.value = true
  try {
    const caches = await loadCacheInventory(getJSON)
    buildRows(caches, supportsOperation('metrics.cache') ? await getText('/metrics') : '')
    loadError.value = ''
  } catch (error) {
    rows.value = []
    loadError.value = error.message
  } finally {
    loading.value = false
  }
}

async function flush(tag) {
  const response = await capabilityFetch(`/plugins/${encodeURIComponent(tag)}/flush`)
  if (!response.ok) throw new Error(`HTTP ${response.status}`)
}

async function clearOne(cache) {
  if (!supportsOperation('cache.manage')) return
  if (!(await openConfirm(`确定要清空缓存“${cache.name}”吗？`, { tone: 'danger' }))) return
  clearing[cache.tag] = true
  try {
    await flush(cache.tag)
    setSuccess(`缓存“${cache.name}”已清空`)
    await load()
  } catch (error) {
    setError(`清空缓存失败: ${error.message}`)
  } finally {
    clearing[cache.tag] = false
  }
}

async function clearAll() {
  if (!supportsOperation('cache.manage')) return
  if (!(await openConfirm(`将依次清空 ${rows.value.length} 个缓存，此操作不可恢复。`, { tone: 'danger' }))) return
  clearingAll.value = true
  try {
    const targets = [...rows.value]
    const results = await Promise.allSettled(targets.map((cache) => flush(cache.tag)))
    const summary = summarizeCacheFlush(targets, results)
    await load()
    if (summary.failed.length > 0) setError(`已清空 ${summary.succeeded} 个缓存；失败：${summary.failed.join('、')}`)
    else setSuccess('全部缓存已清空')
  } finally {
    clearingAll.value = false
  }
}

onMounted(() => {
  load()
  window.addEventListener('mosdns-log-refresh', load)
})
onBeforeUnmount(() => window.removeEventListener('mosdns-log-refresh', load))
</script>

<template>
  <section class="panel rt-cache" data-cache-panel>
    <header class="rt-list-head">
      <div class="rt-list-title"><h3>缓存统计</h3><span class="muted rt-list-count">{{ loading ? '读取中…' : `共 ${rows.length} 个缓存` }}</span></div>
      <div class="actions">
        <button class="btn secondary" type="button" :disabled="loading" @click="load">刷新</button>
        <button class="btn danger" type="button" :disabled="clearingAll || !supportsOperation('cache.manage') || rows.length === 0" @click="clearAll">{{ clearingAll ? '清空中…' : '清空所有缓存' }}</button>
      </div>
    </header>
    <p v-if="!supportsOperation('cache.inventory')" class="muted" role="note">{{ operationReason('cache.inventory') }}</p>
    <p v-else-if="!supportsOperation('cache.manage')" class="muted" role="note">{{ operationReason('cache.manage') }}</p>
    <p v-if="loadError" class="rt-setting-error" role="alert">缓存列表读取失败：{{ loadError }}</p>
    <div class="table-wrap rt-compact-table">
      <table class="cache-adaptive-table">
        <thead>
          <tr><th>缓存</th><th class="text-right">请求总数</th><th class="text-right">命中</th><th class="text-right">过期命中</th><th class="text-right">命中率</th><th class="text-right">过期命中率</th><th class="text-right">条目数</th><th class="rt-col-actions">操作</th></tr>
        </thead>
        <tbody>
          <tr v-if="rows.length === 0"><td colspan="8" class="empty">{{ loadError ? '缓存数据读取失败' : '暂无缓存' }}</td></tr>
          <tr v-for="cache in rows" :key="cache.key">
            <td>{{ cache.name }}</td>
            <td class="text-right">{{ number(cache.query_total) }}</td>
            <td class="text-right">{{ number(cache.hit_total) }}</td>
            <td class="text-right">{{ number(cache.lazy_hit_total) }}</td>
            <td class="text-right">{{ cache.hit_rate }}</td>
            <td class="text-right">{{ cache.lazy_hit_rate }}</td>
            <td class="text-right"><button class="btn-link" type="button" @click="viewing = cache">{{ number(cache.size_current) }}</button></td>
            <td class="rt-col-actions">
              <div class="rt-row-actions">
                <button class="btn tiny danger" type="button" :disabled="Boolean(clearing[cache.tag]) || !supportsOperation('cache.manage')" @click="clearOne(cache)">{{ clearing[cache.tag] ? '清空中…' : '清空' }}</button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <CacheEntriesModal v-if="viewing" :cache="viewing" @close="viewing = null" />
  </section>
</template>
