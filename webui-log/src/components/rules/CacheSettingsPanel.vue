<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { deleteRequest, getJSON, postJSON } from '../../api/http'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { openConfirm } from '../../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'
import { CACHE_RANGES, fieldError, formatCount, formatDuration } from '../../utils/tuning'
import DurationInput from './DurationInput.vue'

// The size, the stale-answer retention and the dump interval of every cache
// (`/api/v1/cache/settings`). A change replaces that cache by an empty one, so saving asks first.
const FIELDS = [
  { key: 'size', label: '容量上限', kind: 'count' },
  { key: 'lazy_cache_ttl', label: '过期缓存保留', kind: 'duration' },
  { key: 'dump_interval', label: '写盘间隔', kind: 'duration' }
]

const rows = ref([])
const loaded = ref(false)
const loadError = ref('')
const busy = reactive({})
const advanced = ref(false)

const allowed = computed(() => supportsOperation('cache.settings'))

function toRow(cache) {
  const server = {}
  const defaults = {}
  for (const { key } of FIELDS) {
    server[key] = Number(cache[key]?.value)
    defaults[key] = Number(cache[key]?.default)
  }
  const label = typeof cache.label === 'string' ? cache.label.trim() : ''
  return { tag: cache.tag, name: label || cache.tag, labelled: label !== '', server, defaults, draft: { ...server } }
}

async function load() {
  if (!allowed.value) return
  try {
    const data = await getJSON('/api/v1/cache/settings')
    rows.value = (Array.isArray(data?.caches) ? data.caches : []).map(toRow)
    loadError.value = ''
  } catch (error) {
    rows.value = []
    loadError.value = error.status === 404 ? '' : `读取缓存参数失败: ${error.message}`
  } finally {
    loaded.value = true
  }
}

const changed = (row) => FIELDS.filter(({ key }) => row.draft[key] !== row.server[key]).map(({ key }) => key)
const modified = (row) => FIELDS.some(({ key }) => row.server[key] !== row.defaults[key])
const isDefault = (row, key) => row.server[key] === row.defaults[key]

function errorOf(row, field) {
  return fieldError(row.draft[field.key], CACHE_RANGES[field.key], field.label, field.kind)
}

const errorsOf = (row) => FIELDS.map((field) => errorOf(row, field)).filter(Boolean)

async function submit(row, action, success) {
  busy[row.tag] = true
  clearTopNotice()
  try {
    const result = await action()
    const reset = Array.isArray(result?.reset) && result.reset.includes(row.tag)
    setSuccess(reset ? `缓存“${row.name}”已按新参数重建，原有内容已清空。` : success)
    await load()
  } catch (error) {
    setError(`保存缓存参数失败: ${error.message}`)
  } finally {
    busy[row.tag] = false
  }
}

async function save(row) {
  const keys = changed(row)
  if (keys.length === 0 || errorsOf(row).length > 0) return
  if (!(await openConfirm(`保存后缓存“${row.name}”会被清空并按新参数重新累积，确定吗？`, { tone: 'danger', confirmText: '保存并清空' }))) return
  const patch = Object.fromEntries(keys.map((key) => [key, row.draft[key]]))
  await submit(row, () => postJSON(`/api/v1/cache/settings/${encodeURIComponent(row.tag)}`, patch), '缓存参数已保存')
}

async function restore(row) {
  if (!(await openConfirm(`恢复默认后缓存“${row.name}”会被清空并按配置文件里的参数重新累积，确定吗？`, { tone: 'danger', confirmText: '恢复并清空' }))) return
  await submit(row, () => deleteRequest(`/api/v1/cache/settings/${encodeURIComponent(row.tag)}`), '已恢复为配置文件里的参数')
}

onMounted(() => {
  load()
  window.addEventListener('mosdns-log-refresh', load)
  window.addEventListener('mosdns-capabilities-ready', load)
})
onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', load)
  window.removeEventListener('mosdns-capabilities-ready', load)
})
</script>

<template>
  <section v-if="!allowed || loadError || rows.length > 0" class="panel rt-cache-settings" data-cache-settings>
    <header class="rt-list-head">
      <div class="rt-list-title"><h3>缓存参数</h3></div>
      <label v-if="allowed && rows.length > 0" class="rt-cache-advanced"><input v-model="advanced" type="checkbox" /> 高级：写盘间隔</label>
    </header>
    <p v-if="!allowed" class="muted rt-setting-note" role="note">{{ operationReason('cache.settings') }}</p>
    <template v-else>
      <p class="muted rt-setting-note" role="note">
        容量上限是最多缓存多少条；过期缓存保留是应答过期后仍可继续提供的时长，0 表示不保留。
        修改后立即生效，<strong>该缓存会被清空</strong>并重新累积；点“恢复默认”回到配置文件里的值。
      </p>
      <p v-if="loadError" class="rt-setting-error" role="alert">{{ loadError }}</p>
      <div class="table-wrap rt-compact-table">
        <table>
          <thead>
            <tr>
              <th>缓存</th>
              <th>容量上限（条）</th>
              <th>过期缓存保留</th>
              <th v-if="advanced">写盘间隔</th>
              <th class="rt-col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in rows" :key="row.tag" :data-cache-row="row.tag">
              <td>
                <div>{{ row.name }}</div>
                <span v-if="row.labelled" class="rt-tag-note">{{ row.tag }}</span>
                <span v-if="modified(row)" class="rt-flag-mark" :data-cache-modified="row.tag">已修改</span>
              </td>
              <td>
                <input v-model.number="row.draft.size" class="rt-setting-input" type="number" min="1000" step="1000" :disabled="busy[row.tag]" :aria-label="`${row.name} 容量上限`" />
                <p v-if="errorOf(row, FIELDS[0])" class="rt-setting-error" role="alert">{{ errorOf(row, FIELDS[0]) }}</p>
                <p v-else-if="!isDefault(row, 'size')" class="muted rt-setting-note">默认 {{ formatCount(row.defaults.size) }}</p>
              </td>
              <td>
                <DurationInput v-model="row.draft.lazy_cache_ttl" :label="`${row.name} 过期缓存保留`" :disabled="busy[row.tag]" />
                <p v-if="errorOf(row, FIELDS[1])" class="rt-setting-error" role="alert">{{ errorOf(row, FIELDS[1]) }}</p>
                <p v-else-if="row.draft.lazy_cache_ttl === 0" class="muted rt-setting-note">不保留过期缓存</p>
                <p v-else-if="row.draft.lazy_cache_ttl >= 1000 * 86400" class="muted rt-setting-note">{{ formatDuration(row.draft.lazy_cache_ttl) }}，几乎不限</p>
                <p v-else-if="!isDefault(row, 'lazy_cache_ttl')" class="muted rt-setting-note">默认 {{ formatDuration(row.defaults.lazy_cache_ttl) }}</p>
              </td>
              <td v-if="advanced">
                <DurationInput v-model="row.draft.dump_interval" :label="`${row.name} 写盘间隔`" :disabled="busy[row.tag]" />
                <p v-if="errorOf(row, FIELDS[2])" class="rt-setting-error" role="alert">{{ errorOf(row, FIELDS[2]) }}</p>
                <p v-else-if="!isDefault(row, 'dump_interval')" class="muted rt-setting-note">默认 {{ formatDuration(row.defaults.dump_interval) }}</p>
              </td>
              <td class="rt-col-actions">
                <div class="rt-row-actions">
                  <button class="btn primary" type="button" :disabled="busy[row.tag] || changed(row).length === 0 || errorsOf(row).length > 0" @click="save(row)">保存</button>
                  <button v-if="modified(row)" class="btn secondary" type="button" :disabled="busy[row.tag]" @click="restore(row)">恢复默认</button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>
  </section>
</template>
