<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { getJSON, putJSON } from '../../api/http'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { aliasMapFromRows, aliasRowsFrom, aliasesChanged, normalizeAliasMap, normalizeIP, sortAliasRows } from '../../utils/clientAliases'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'
import { clearDirty, setDirty } from '../../utils/unsaved'

// Names for client addresses; the log pages show and search by them.
const uid = `aliases:${Math.random().toString(36).slice(2)}`
const rows = ref([])
const saved = ref({})
const loading = ref(false)
const saving = ref(false)
const manualIp = ref('')
const manualName = ref('')
const filter = ref('')
const importInput = ref(null)
// The saved aliases were read; until then nothing can be saved (a save replaces them all).
const loaded = ref(false)
const loadError = ref('')

const allowed = computed(() => supportsOperation('client.aliases'))
const dirty = computed(() => aliasesChanged(rows.value))
const shown = computed(() => {
  const needle = filter.value.trim().toLowerCase()
  return needle ? rows.value.filter((row) => row.ip.toLowerCase().includes(needle) || row.alias.toLowerCase().includes(needle)) : rows.value
})

watch(dirty, (value) => setDirty(uid, value), { immediate: true })

async function load() {
  if (!allowed.value || dirty.value || saving.value) return
  loading.value = true
  try {
    const [aliases, clients] = await Promise.all([
      getJSON('/plugins/clientname'),
      supportsOperation('query.rank') ? getJSON('/api/v2/audit/rank/client?limit=200').catch(() => []) : Promise.resolve([])
    ])
    saved.value = normalizeAliasMap(aliases)
    rows.value = aliasRowsFrom(saved.value, Array.isArray(clients) ? clients.map((item) => item?.key) : [])
    loaded.value = true
    loadError.value = ''
  } catch (error) {
    loadError.value = `读取别名失败: ${error.message}`
  } finally {
    loading.value = false
  }
}

async function put(map) {
  if (!loaded.value) throw new Error('别名还没有成功读取')
  await putJSON('/plugins/clientname', map)
  saved.value = normalizeAliasMap(map)
  rows.value = aliasRowsFrom(saved.value, rows.value.map((row) => row.ip))
}

async function save() {
  clearTopNotice()
  saving.value = true
  try {
    await put(aliasMapFromRows(rows.value))
    setSuccess('客户端别名已保存')
  } catch (error) {
    setError(`保存别名失败: ${error.message}`)
  } finally {
    saving.value = false
  }
}

function addManual() {
  if (!loaded.value) return setError('别名还没有成功读取，请先重新读取')
  const ip = normalizeIP(manualIp.value)
  const alias = manualName.value.trim()
  if (!ip || !alias) return setError('请填写完整的 IP 与别名')
  const existing = rows.value.find((row) => normalizeIP(row.ip) === ip)
  if (existing) existing.alias = alias
  else rows.value = sortAliasRows([...rows.value, { ip, alias, originalAlias: '' }])
  manualIp.value = ''
  manualName.value = ''
  setSuccess(`已加入待保存项: ${ip}`)
}

function readJson(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      try {
        resolve(JSON.parse(String(reader.result || '{}')))
      } catch (error) {
        reject(error)
      }
    }
    reader.onerror = () => reject(new Error('读取文件失败'))
    reader.readAsText(file)
  })
}

async function importFile(event) {
  const file = event?.target?.files?.[0]
  if (!file) return
  try {
    const imported = normalizeAliasMap(await readJson(file))
    if (Object.keys(imported).length === 0) throw new Error('文件中没有可用的别名数据')
    await put({ ...saved.value, ...imported })
    setSuccess(`已导入 ${Object.keys(imported).length} 条别名`)
  } catch (error) {
    setError(`导入别名失败: ${error.message}`)
  } finally {
    event.target.value = ''
  }
}

async function exportFile() {
  try {
    const latest = normalizeAliasMap(await getJSON('/plugins/clientname'))
    const url = URL.createObjectURL(new Blob([JSON.stringify(latest, null, 2)], { type: 'application/json' }))
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `rtdns-aliases-${new Date().toISOString().slice(0, 10)}.json`
    document.body.appendChild(anchor)
    anchor.click()
    document.body.removeChild(anchor)
    URL.revokeObjectURL(url)
  } catch (error) {
    setError(`导出别名失败: ${error.message}`)
  }
}

onMounted(() => {
  load()
  window.addEventListener('mosdns-log-refresh', load)
})
onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', load)
  clearDirty(uid)
})
</script>

<template>
  <section class="panel rt-aliases" data-aliases>
    <header class="rt-list-head">
      <div class="rt-list-title">
        <h3>客户端别名</h3>
        <span class="muted rt-list-count">{{ loading ? '读取中…' : `共 ${Object.keys(saved).length} 个别名` }}</span>
        <span v-if="dirty" class="rt-flag-mark" data-aliases-dirty>未保存</span>
      </div>
      <input v-model="filter" class="rt-setting-input rt-list-search" type="search" placeholder="搜索地址或别名" aria-label="搜索客户端" />
    </header>
    <p class="muted rt-list-hint">给客户端地址起名字，日志里就显示名字，也可以按名字搜索。下面列出最近出现过的客户端；别名留空表示删除。</p>
    <p v-if="!allowed" class="muted" role="note">{{ operationReason('client.aliases') }}</p>
    <template v-else>
      <p v-if="loadError" class="rt-setting-error" role="alert" data-aliases-error>{{ loadError }}（读取成功之前不能保存，以免覆盖已有的别名）<button class="btn-link" type="button" @click="load">重新读取</button></p>
      <div class="rt-alias-add">
        <input v-model="manualIp" class="rt-setting-input" placeholder="客户端 IP，例如 192.0.2.10" aria-label="客户端 IP" />
        <input v-model="manualName" class="rt-setting-input" placeholder="别名，例如 iPhone" aria-label="别名" @keyup.enter="addManual" />
        <button class="btn secondary" type="button" @click="addManual">加入列表</button>
      </div>
      <p v-if="loading && rows.length === 0" class="muted">读取中…</p>
      <p v-else-if="shown.length === 0" class="muted">{{ filter ? '没有匹配的客户端' : '暂无客户端' }}</p>
      <ul v-else class="rt-alias-grid">
        <li v-for="row in shown" :key="`alias-${row.ip}`" class="rt-alias-item">
          <span class="mono rt-alias-ip" :title="row.ip">{{ row.ip }}</span>
          <input v-model="row.alias" class="rt-setting-input rt-alias-input" placeholder="为空表示删除别名" :aria-label="`${row.ip} 的别名`" />
        </li>
      </ul>
      <div class="actions rt-list-actions">
        <button class="btn primary" type="button" :disabled="loading || saving || !loaded || !dirty" @click="save">{{ saving ? '保存中…' : '保存全部' }}</button>
        <button class="btn secondary" type="button" :disabled="loading || saving" @click="exportFile">导出 JSON</button>
        <button class="btn secondary" type="button" :disabled="loading || saving || !loaded || dirty" :title="dirty ? '先保存或还原当前改动' : ''" @click="importInput.click()">导入 JSON</button>
        <input ref="importInput" type="file" accept=".json,application/json" style="display: none" @change="importFile" />
      </div>
    </template>
  </section>
</template>
