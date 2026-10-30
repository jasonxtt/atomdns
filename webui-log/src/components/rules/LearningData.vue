<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { getJSON, getText, postJSON } from '../../api/http'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { openConfirm } from '../../utils/confirm'
import { setError, setSuccess } from '../../utils/notice'
import { formatDateTime } from '../../utils/time'
import { LEARNING_RANGES, fieldError, formatCount, formatDuration } from '../../utils/tuning'
import AppIcon from '../AppIcon.vue'
import DurationInput from './DurationInput.vue'

// What the host has learned: the categories with their counts (and clearing a whole
// category), and the entries with why and when they were learned. Entries cannot be
// deleted one by one. The validity and capacity of every category and the write interval
// are edited here too (`/api/v1/learning/settings`); changing them never clears anything.
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

const tunable = computed(() => supportsOperation('learning.settings'))
// The saved numbers (with their defaults) and what is typed in the fields.
const tuning = ref(null)
const drafts = reactive({})
const interval = reactive({ server: 0, default: 0, draft: 0 })
const tuningBusy = ref('')

const serverOf = (tag) => {
  const found = tuning.value?.categories?.find((item) => item.tag === tag)
  return found ? { ttl: Number(found.ttl.value), max_entries: Number(found.max_entries.value), defaults: { ttl: Number(found.ttl.default), max_entries: Number(found.max_entries.default) } } : null
}
const changedKeys = (tag) => {
  const server = serverOf(tag)
  const draft = drafts[tag]
  return server && draft ? ['ttl', 'max_entries'].filter((key) => draft[key] !== server[key]) : []
}
const modifiedCategory = (tag) => {
  const server = serverOf(tag)
  return Boolean(server) && (server.ttl !== server.defaults.ttl || server.max_entries !== server.defaults.max_entries)
}
const categoryErrors = (tag) => {
  const draft = drafts[tag]
  if (!draft) return []
  return [
    fieldError(draft.ttl, LEARNING_RANGES.ttl, '有效期', 'duration'),
    fieldError(draft.max_entries, LEARNING_RANGES.max_entries, '容量上限', 'count')
  ].filter(Boolean)
}
const intervalError = computed(() => fieldError(interval.draft, LEARNING_RANGES.write_interval, '写盘间隔', 'duration'))

async function loadTuning() {
  if (!tunable.value) return
  try {
    const data = await getJSON('/api/v1/learning/settings')
    tuning.value = data
    for (const item of data.categories || []) drafts[item.tag] = { ttl: Number(item.ttl.value), max_entries: Number(item.max_entries.value) }
    interval.server = Number(data.write_interval?.value)
    interval.default = Number(data.write_interval?.default)
    interval.draft = interval.server
  } catch (error) {
    tuning.value = null
    if (error.status !== 404) setError(`读取学习参数失败: ${error.message}`)
  }
}

async function applyTuning(key, body, success) {
  tuningBusy.value = key
  try {
    await postJSON('/api/v1/learning/settings', body)
    setSuccess(success)
    await Promise.all([loadTuning(), loadStatus()])
  } catch (error) {
    setError(`保存学习参数失败: ${error.message}`)
  } finally {
    tuningBusy.value = ''
  }
}

function saveCategory(item) {
  const keys = changedKeys(item.tag)
  if (keys.length === 0 || categoryErrors(item.tag).length > 0) return Promise.resolve()
  const patch = Object.fromEntries(keys.map((key) => [key, drafts[item.tag][key]]))
  return applyTuning(item.tag, { categories: { [item.tag]: patch } }, `“${item.label}”的参数已保存并立即生效`)
}

function restoreCategory(item) {
  return applyTuning(item.tag, { categories: { [item.tag]: { ttl: null, max_entries: null } } }, `“${item.label}”已恢复为配置文件里的参数`)
}

function saveInterval() {
  if (interval.draft === interval.server || intervalError.value) return Promise.resolve()
  return applyTuning('interval', { write_interval: interval.draft }, '写盘间隔已保存并立即生效')
}

function restoreInterval() {
  return applyTuning('interval', { write_interval: null }, '写盘间隔已恢复为配置文件里的值')
}

const counters = computed(() => status.value?.counters || null)
const lastWrite = computed(() => status.value?.last_write || null)

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
  await Promise.all([loadEntries(), loadTuning()])
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
  window.addEventListener('atomdns-log-refresh', reload)
})
onBeforeUnmount(() => {
  window.clearTimeout(searchTimer)
  window.removeEventListener('atomdns-log-refresh', reload)
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
      <div v-if="tuning" class="rt-learning-interval" data-learning-interval>
        <span class="rt-learning-interval-label">写盘间隔</span>
        <DurationInput v-model="interval.draft" label="写盘间隔" :disabled="tuningBusy === 'interval'" />
        <span v-if="interval.server !== interval.default" class="rt-flag-mark" data-learning-modified="interval">已修改</span>
        <button class="btn primary" type="button" :disabled="tuningBusy === 'interval' || interval.draft === interval.server || Boolean(intervalError)" @click="saveInterval">保存</button>
        <button v-if="interval.server !== interval.default" class="btn secondary" type="button" :disabled="tuningBusy === 'interval'" @click="restoreInterval">恢复默认</button>
        <span class="muted rt-setting-note">学到的内容有变化时，每隔这么久写一次磁盘（默认 {{ formatDuration(interval.default) }}）。</span>
        <p v-if="intervalError" class="rt-setting-error" role="alert">{{ intervalError }}</p>
      </div>
      <div class="table-wrap rt-compact-table">
        <table class="rt-learning-categories" data-learning-categories>
          <thead>
            <tr><th>类别</th><th class="text-right">条数</th><th>上限</th><th>有效期</th><th>状态</th><th class="rt-col-actions">操作</th></tr>
          </thead>
          <tbody>
            <tr v-if="categories.length === 0"><td colspan="6" class="empty">暂无类别</td></tr>
            <tr v-for="item in categories" :key="item.tag" :class="{ disabled: !item.enabled }">
              <td>{{ item.label }}</td>
              <td class="text-right">{{ Number(item.entries).toLocaleString() }}</td>
              <td>
                <template v-if="drafts[item.tag]">
                  <input v-model.number="drafts[item.tag].max_entries" class="rt-setting-input rt-setting-input-short" type="number" min="1" :disabled="tuningBusy === item.tag" :aria-label="`${item.label} 容量上限`" />
                </template>
                <template v-else>{{ Number(item.max_entries).toLocaleString() }}</template>
              </td>
              <td>
                <template v-if="drafts[item.tag]">
                  <DurationInput v-model="drafts[item.tag].ttl" :label="`${item.label} 有效期`" :disabled="tuningBusy === item.tag" />
                  <span v-if="modifiedCategory(item.tag)" class="rt-flag-mark" :data-learning-modified="item.tag">已修改</span>
                  <p v-for="message in categoryErrors(item.tag)" :key="message" class="rt-setting-error" role="alert">{{ message }}</p>
                </template>
                <template v-else>{{ ttlText(item.ttl_seconds) }}</template>
              </td>
              <td>{{ item.enabled ? '学习中' : '已关闭' }}</td>
              <td class="rt-col-actions">
                <div class="rt-row-actions">
                  <button v-if="drafts[item.tag]" class="btn primary" type="button" :disabled="tuningBusy === item.tag || changedKeys(item.tag).length === 0 || categoryErrors(item.tag).length > 0" @click="saveCategory(item)">保存</button>
                  <button v-if="drafts[item.tag] && modifiedCategory(item.tag)" class="btn secondary" type="button" :disabled="tuningBusy === item.tag" @click="restoreCategory(item)">恢复默认</button>
                  <button class="rt-row-icon danger" type="button" title="清空" :aria-label="`清空 ${item.label || item.tag}`" :disabled="busy === item.tag || Number(item.entries) === 0" @click="flush(item)">
                    <AppIcon name="trash" :size="16" />
                  </button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <p v-if="tuning" class="muted rt-setting-note" role="note">有效期从最后一次确认起算：调小会让部分条目立刻过期，调大让现有条目活得更久；容量调小时保留最近确认的条目。保存后立即生效，不会清空其余内容。</p>
      <p v-if="counters" class="muted rt-setting-note" data-learning-counters>
        自启动以来：已学习 {{ formatCount(counters.learned) }} · 已确认 {{ formatCount(counters.confirmed) }} · 已过期 {{ formatCount(counters.expired) }} · 已淘汰 {{ formatCount(counters.evicted) }} · 已清空 {{ formatCount(counters.flushed) }}；
        <template v-if="lastWrite">上次写盘 {{ formatDateTime(lastWrite.at) }}（{{ formatCount(lastWrite.entries) }} 条）<template v-if="lastWrite.error">，失败：{{ lastWrite.error }}</template></template>
        <template v-else>尚未写过盘</template>
      </p>

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
              <td :title="item.reason">{{ item.reason_label || item.reason }}</td>
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
