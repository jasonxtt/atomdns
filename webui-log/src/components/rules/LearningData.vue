<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { getJSON, getText } from '../../api/http'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { openConfirm } from '../../utils/confirm'
import { setError, setSuccess } from '../../utils/notice'
import { formatDateTime } from '../../utils/time'

// What the host has learned: the categories with their counts (and clearing a whole
// category), and the entries with why and when they were learned. Entries cannot be
// deleted one by one.
const PAGE = 50
const status = ref(null)
const statusError = ref('')
const entries = ref([])
const total = ref(0)
const entriesError = ref('')
const loading = ref(false)
const category = ref('')
const search = ref('')
const sort = ref('hits')
const offset = ref(0)
const busy = ref('')
let searchTimer = 0

const allowed = computed(() => supportsOperation('lists.remembered'))
const categories = computed(() => (Array.isArray(status.value?.categories) ? status.value.categories : []))
const labelOf = (tag) => categories.value.find((item) => item.tag === tag)?.label || tag
const pageNumber = computed(() => Math.floor(offset.value / PAGE) + 1)
const lastPage = computed(() => Math.max(1, Math.ceil(total.value / PAGE)))

function ttlText(seconds) {
  const days = Number(seconds || 0) / 86400
  return days >= 1 ? `${Math.round(days * 10) / 10} 天` : `${Math.round(Number(seconds || 0) / 3600)} 小时`
}

async function loadStatus() {
  if (!allowed.value) return
  try {
    status.value = await getJSON('/plugins/route_learner/status')
    statusError.value = ''
  } catch (error) {
    statusError.value = error.status === 404 ? '当前配置没有域名学习功能。' : `读取学习状态失败: ${error.message}`
  }
}

async function loadEntries() {
  if (!allowed.value || statusError.value) return
  loading.value = true
  try {
    const params = new URLSearchParams({ limit: String(PAGE), offset: String(offset.value), sort: sort.value, order: 'desc' })
    if (category.value) params.set('category', category.value)
    if (search.value.trim()) params.set('q', search.value.trim())
    const data = await getJSON(`/plugins/route_learner/entries?${params.toString()}`)
    entries.value = Array.isArray(data?.entries) ? data.entries : []
    total.value = Number(data?.total || 0)
    entriesError.value = ''
  } catch (error) {
    entries.value = []
    entriesError.value = `读取学习明细失败: ${error.message}`
  } finally {
    loading.value = false
  }
}

async function reload() {
  await loadStatus()
  await loadEntries()
}

function goto(page) {
  offset.value = Math.max(0, (Math.min(lastPage.value, Math.max(1, page)) - 1) * PAGE)
  loadEntries()
}

watch([category, sort], () => {
  offset.value = 0
  loadEntries()
})
watch(search, () => {
  window.clearTimeout(searchTimer)
  searchTimer = window.setTimeout(() => {
    offset.value = 0
    loadEntries()
  }, 350)
})

async function flush(item) {
  if (!(await openConfirm(`确定清空“${item.label}”吗？已学到的 ${Number(item.entries).toLocaleString()} 条会全部丢失。`, { tone: 'danger', confirmText: '清空' }))) return
  busy.value = item.tag
  try {
    await getText(`/plugins/${encodeURIComponent(item.tag)}/flush`)
    setSuccess(`“${item.label}”已清空`)
    await reload()
  } catch (error) {
    setError(`清空失败: ${error.message}`)
  } finally {
    busy.value = ''
  }
}

async function saveNow() {
  busy.value = 'save'
  try {
    const results = await Promise.allSettled(categories.value.map((item) => getText(`/plugins/${encodeURIComponent(item.tag)}/save`)))
    const failed = results.filter((item) => item.status === 'rejected').length
    if (failed > 0) setError(`部分类别保存失败 (${failed}/${results.length})`)
    else setSuccess('已把学到的内容保存到磁盘')
    await loadStatus()
  } finally {
    busy.value = ''
  }
}

onMounted(() => {
  reload()
  window.addEventListener('mosdns-log-refresh', reload)
})
onBeforeUnmount(() => {
  window.clearTimeout(searchTimer)
  window.removeEventListener('mosdns-log-refresh', reload)
})
</script>

<template>
  <section class="panel rt-learning" data-learning>
    <header class="rt-list-head">
      <div class="rt-list-title"><h3>学到的内容</h3></div>
      <div class="actions" v-if="allowed && !statusError">
        <button class="btn secondary" type="button" :disabled="busy === 'save'" @click="saveNow">{{ busy === 'save' ? '保存中…' : '立即保存到磁盘' }}</button>
      </div>
    </header>
    <p v-if="!allowed" class="muted" role="note">{{ operationReason('lists.remembered') }}</p>
    <p v-else-if="statusError" class="muted" role="note">{{ statusError }}</p>
    <template v-else>
      <div class="table-wrap rt-compact-table">
        <table class="rt-learning-categories" data-learning-categories>
          <thead>
            <tr><th>类别</th><th class="text-right">条数</th><th class="text-right">上限</th><th>有效期</th><th>状态</th><th class="rt-col-actions">操作</th></tr>
          </thead>
          <tbody>
            <tr v-if="categories.length === 0"><td colspan="6" class="empty">暂无类别</td></tr>
            <tr v-for="item in categories" :key="item.tag" :class="{ disabled: !item.enabled }">
              <td>{{ item.label }}</td>
              <td class="text-right">{{ Number(item.entries).toLocaleString() }}</td>
              <td class="text-right">{{ Number(item.max_entries).toLocaleString() }}</td>
              <td>{{ ttlText(item.ttl_seconds) }}</td>
              <td>{{ item.enabled ? '学习中' : '已关闭' }}</td>
              <td class="rt-col-actions">
                <div class="rt-row-actions">
                  <button class="btn tiny danger" type="button" :disabled="busy === item.tag || Number(item.entries) === 0" @click="flush(item)">清空</button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="rt-list-head rt-learning-filter">
        <div class="rt-list-title"><h3>学习明细</h3><span class="muted rt-list-count" data-entries-total>共 {{ total.toLocaleString() }} 条</span></div>
        <div class="rt-learning-controls">
          <select v-model="category" class="rt-setting-input" aria-label="按类别筛选">
            <option value="">全部类别</option>
            <option v-for="item in categories" :key="item.tag" :value="item.tag">{{ item.label }}</option>
          </select>
          <select v-model="sort" class="rt-setting-input" aria-label="排序">
            <option value="hits">按命中次数</option>
            <option value="last">按最近确认</option>
            <option value="first">按首次确认</option>
          </select>
          <input v-model="search" class="rt-setting-input" type="search" placeholder="搜索域名" aria-label="搜索域名" />
        </div>
      </div>
      <p v-if="entriesError" class="rt-setting-error" role="alert">{{ entriesError }}</p>
      <div class="table-wrap rt-compact-table rt-scroll-wrap">
        <table class="rt-learning-entries" data-learning-entries>
          <thead>
            <tr><th>域名</th><th>类别</th><th>原因</th><th>首次确认</th><th>最近确认</th><th>到期</th><th class="text-right">命中</th></tr>
          </thead>
          <tbody>
            <tr v-if="loading && entries.length === 0"><td colspan="7" class="empty">读取中…</td></tr>
            <tr v-else-if="entries.length === 0"><td colspan="7" class="empty">没有匹配的条目</td></tr>
            <tr v-for="item in entries" :key="`${item.category}:${item.name}`">
              <td class="mono" :title="item.name">{{ item.name }}</td>
              <td>{{ labelOf(item.category) }}</td>
              <td :title="item.reason">{{ item.reason }}</td>
              <td class="mono">{{ formatDateTime(item.first_confirmed) }}</td>
              <td class="mono">{{ formatDateTime(item.last_confirmed) }}</td>
              <td class="mono">{{ formatDateTime(item.expires_at) }}</td>
              <td class="text-right">{{ Number(item.hits).toLocaleString() }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div class="actions rt-pager">
        <button class="btn secondary" type="button" :disabled="pageNumber <= 1 || loading" @click="goto(pageNumber - 1)">上一页</button>
        <span class="muted rt-pager-text">第 {{ pageNumber }} / {{ lastPage }} 页</span>
        <button class="btn secondary" type="button" :disabled="pageNumber >= lastPage || loading" @click="goto(pageNumber + 1)">下一页</button>
      </div>
    </template>
  </section>
</template>
