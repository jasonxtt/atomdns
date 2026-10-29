<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { deleteRequest, getJSON, postJSON, putJSON } from '../../api/http'
import { loadNativeOnlineCatalogs } from '../../api/nativeManagement'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { openConfirm } from '../../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'
import { SUBSCRIPTION_FORMATS, inferFromUrl, isDownloaded, needsCategory, ruleFields, sortRules, withFormat } from '../../utils/subscription'
import { formatDateTime } from '../../utils/time'

// The one table for online subscriptions and AdGuard rules: name, address, enabled,
// rule count, last update, manual update, edit / add / delete.
//   kind 'adguard'  the AdGuard rule list (`/plugins/adguard/rules`)
//   kind 'online'   the downloadable sets of `catalogs` (`/plugins/<tag>/config`): the five
//                   sets of 分流规则, or the subscriptions that point at one special group
const props = defineProps({
  kind: { type: String, required: true },
  title: { type: String, required: true },
  catalogs: { type: Array, default: () => [] },
  hint: { type: String, default: '' }
})

const adguard = computed(() => props.kind === 'adguard')
const showType = computed(() => !adguard.value && props.catalogs.length > 1)
const operation = computed(() => (adguard.value ? 'rules.adguard' : 'rules.diversion'))
const allowed = computed(() => supportsOperation(operation.value))

const rules = ref([])
const loading = ref(false)
const loadError = ref('')
const pendingUpdate = ref(false)
let refreshTimer = 0

const editor = reactive({
  open: false,
  saving: false,
  editing: false,
  readOnlyType: false,
  id: '',
  oldName: '',
  type: '',
  name: '',
  url: '',
  files: '',
  format: 'auto',
  category: '',
  enabled: true,
  auto_update: true,
  enable_regexp: false,
  update_interval_hours: 24
})
let raw = null
const touched = reactive({ name: false, files: false })

const tagOf = (type) => props.catalogs.find((item) => item.type === type)?.tag || ''
const itemPath = (tag, name) => `/plugins/${encodeURIComponent(tag)}/config/${encodeURIComponent(name)}`
const showCategory = computed(() => needsCategory(editor.format, editor.url))
const empty = computed(() => (adguard.value ? '暂无 AdGuard 规则' : '暂无订阅'))
const colSpan = 6

async function load() {
  if (!allowed.value) return
  loading.value = true
  loadError.value = ''
  try {
    if (adguard.value) {
      const data = await getJSON('/plugins/adguard/rules')
      rules.value = (Array.isArray(data) ? data : []).sort((a, b) => String(a.name || '').localeCompare(String(b.name || '')))
      return
    }
    const result = await loadNativeOnlineCatalogs(props.catalogs.map((item) => [item.type, item.tag]), getJSON)
    if (result.failures.length > 0) loadError.value = `部分订阅读取失败: ${result.failures.map(({ tag, error }) => `${tag}: ${error}`).join('；')}`
    const merged = []
    result.catalogs.forEach(({ type, tag, rules: list }) => {
      const label = props.catalogs.find((item) => item.type === type)?.label || type
      list.forEach((rule) => {
        merged.push({ ...rule, type: rule.type || type, __tag: tag, __label: label })
      })
    })
    rules.value = sortRules(merged)
  } catch (error) {
    rules.value = []
    loadError.value = `读取失败: ${error.message}`
  } finally {
    loading.value = false
  }
}

// An update runs in the background; the list is read again shortly after.
function reloadSoon() {
  pendingUpdate.value = true
  window.clearTimeout(refreshTimer)
  refreshTimer = window.setTimeout(() => {
    pendingUpdate.value = false
    load()
  }, 5000)
}

function resetEditor() {
  Object.assign(editor, {
    open: true, saving: false, editing: false, readOnlyType: false, id: '', oldName: '', type: props.catalogs[0]?.type || '',
    name: '', url: '', files: '', format: 'auto', category: '', enabled: true, auto_update: true, enable_regexp: false, update_interval_hours: 24
  })
  touched.name = false
  touched.files = false
  raw = null
}

function beginAdd() {
  clearTopNotice()
  resetEditor()
}

function beginEdit(rule) {
  clearTopNotice()
  resetEditor()
  raw = rule
  Object.assign(editor, {
    editing: true,
    readOnlyType: true,
    id: String(rule.id || ''),
    oldName: String(rule.name || ''),
    type: String(rule.type || ''),
    name: String(rule.name || ''),
    url: String(rule.url || ''),
    files: String(rule.files || ''),
    format: String(rule.format || 'auto'),
    category: String(rule.category || ''),
    enabled: Boolean(rule.enabled),
    auto_update: Boolean(rule.auto_update),
    enable_regexp: Boolean(rule.enable_regexp),
    update_interval_hours: Number(rule.update_interval_hours || 24)
  })
}

function onUrlInput() {
  if (editor.editing || adguard.value) return
  const guess = inferFromUrl(editor.url)
  if (!guess) return
  if (!touched.name || !editor.name.trim()) editor.name = guess.name
  if (!touched.files || !editor.files.trim()) editor.files = guess.files
}

function adguardPayload() {
  const name = editor.name.trim()
  const url = editor.url.trim()
  if (!name || !url) throw new Error('名称和地址不能为空')
  return { name, url, enabled: Boolean(editor.enabled), auto_update: Boolean(editor.auto_update), update_interval_hours: Number(editor.update_interval_hours || 24) }
}

function diversionPayload() {
  const name = editor.name.trim()
  const files = editor.files.trim()
  const url = editor.url.trim()
  if (!name) throw new Error('名称不能为空')
  if (!files || !url) throw new Error('本地文件和地址都不能为空')
  return {
    name, type: editor.type, files, url,
    enabled: Boolean(editor.enabled), auto_update: Boolean(editor.auto_update), enable_regexp: Boolean(editor.enable_regexp),
    update_interval_hours: Number(editor.update_interval_hours || 24)
  }
}

async function waitDownloaded(tag, name) {
  for (let attempt = 0; attempt < 6; attempt += 1) {
    await load()
    if (isDownloaded(rules.value.find((rule) => rule.name === name))) return true
    await new Promise((resolve) => window.setTimeout(resolve, 3000))
  }
  return false
}

async function save() {
  editor.saving = true
  try {
    if (adguard.value) {
      const payload = adguardPayload()
      if (editor.id) await putJSON(`/plugins/adguard/rules/${encodeURIComponent(editor.id)}`, withFormat(ruleFields(raw), payload, editor.format, editor.category))
      else await postJSON('/plugins/adguard/rules', withFormat({}, payload, editor.format, editor.category))
      setSuccess(editor.id ? 'AdGuard 规则已更新' : 'AdGuard 规则已新增')
      editor.open = false
      await load()
      return
    }
    const payload = diversionPayload()
    const tag = tagOf(editor.type)
    if (!tag) throw new Error(`无效的类型: ${editor.type}`)
    await putJSON(itemPath(tag, payload.name), withFormat(ruleFields(raw), payload, editor.format, editor.category))
    // A renamed record is a new record: the old one goes.
    if (editor.oldName && editor.oldName !== payload.name) await deleteRequest(itemPath(tag, editor.oldName))
    setSuccess(editor.editing ? '订阅已更新' : '订阅已新增')
    editor.open = false
    await load()
    if (!editor.editing && payload.url) {
      setSuccess(`正在后台下载“${payload.name}”…`)
      try {
        await postJSON(`/plugins/${encodeURIComponent(tag)}/update/${encodeURIComponent(payload.name)}`, {})
        setSuccess((await waitDownloaded(tag, payload.name)) ? `“${payload.name}”已下载` : `“${payload.name}”已开始下载，稍后刷新可查看条数`)
      } catch (error) {
        setError(`触发下载失败: ${error.message}`)
      }
    }
  } catch (error) {
    setError(`保存失败: ${error.message}`)
  } finally {
    editor.saving = false
  }
}

async function toggle(rule) {
  try {
    if (adguard.value) await putJSON(`/plugins/adguard/rules/${encodeURIComponent(rule.id)}`, { ...ruleFields(rule), enabled: !rule.enabled })
    else await putJSON(itemPath(rule.__tag, rule.name), { ...ruleFields(rule), enabled: !rule.enabled })
    await load()
  } catch (error) {
    setError(`切换失败: ${error.message}`)
    await load()
  }
}

async function remove(rule) {
  if (!(await openConfirm(`确定删除“${rule.name}”吗？`, { tone: 'danger' }))) return
  try {
    if (adguard.value) await deleteRequest(`/plugins/adguard/rules/${encodeURIComponent(rule.id)}`)
    else await deleteRequest(itemPath(rule.__tag, rule.name))
    setSuccess('已删除')
    await load()
  } catch (error) {
    setError(`删除失败: ${error.message}`)
  }
}

async function updateOne(rule) {
  try {
    if (adguard.value) await postJSON(`/plugins/adguard/update/${encodeURIComponent(rule.id)}`, {})
    else await postJSON(`/plugins/${encodeURIComponent(rule.__tag)}/update/${encodeURIComponent(rule.name)}`, {})
    setSuccess(`已触发“${rule.name}”更新，5 秒后自动刷新`)
    reloadSoon()
  } catch (error) {
    setError(`触发更新失败: ${error.message}`)
  }
}

async function updateAll() {
  try {
    if (adguard.value) {
      await postJSON('/plugins/adguard/update', {})
      setSuccess('已触发全部更新，5 秒后自动刷新')
    } else {
      const targets = rules.value.filter((rule) => rule.__tag && rule.name)
      if (targets.length === 0) return setError('暂无可更新的订阅')
      const settled = await Promise.allSettled(targets.map((rule) => postJSON(`/plugins/${encodeURIComponent(rule.__tag)}/update/${encodeURIComponent(rule.name)}`, {})))
      const failed = settled.filter((item) => item.status === 'rejected').length
      if (failed === settled.length) return setError(`触发更新失败，共 ${failed} 条`)
      if (failed > 0) setError(`已触发 ${settled.length - failed} 条更新，${failed} 条触发失败`)
      else setSuccess(`已触发全部 ${settled.length} 条更新，5 秒后自动刷新`)
    }
    reloadSoon()
  } catch (error) {
    setError(`触发更新失败: ${error.message}`)
  }
}

onMounted(() => {
  load()
  window.addEventListener('mosdns-log-refresh', load)
})
onBeforeUnmount(() => {
  window.clearTimeout(refreshTimer)
  window.removeEventListener('mosdns-log-refresh', load)
})
</script>

<template>
  <section class="panel rt-subscriptions" :data-subscriptions="kind">
    <header class="rt-list-head">
      <div class="rt-list-title">
        <h3>{{ title }}</h3>
        <span class="muted rt-list-count">{{ loading ? '读取中…' : `共 ${rules.length} 条` }}</span>
      </div>
      <div class="actions" v-if="allowed">
        <button class="btn primary" type="button" @click="beginAdd">新增</button>
        <button class="btn secondary" type="button" :disabled="pendingUpdate || rules.length === 0" @click="updateAll">更新全部</button>
      </div>
    </header>
    <p v-if="hint" class="muted rt-list-hint">{{ hint }}</p>
    <p v-if="!allowed" class="muted" role="note">{{ operationReason(operation) }}</p>
    <template v-else>
      <p v-if="loadError" class="rt-setting-error" role="alert">{{ loadError }}</p>
      <div class="table-wrap rt-compact-table rt-sub-wrap">
        <table class="rt-sub-table">
          <thead>
            <tr>
              <th class="rt-col-switch">启用</th>
              <th>名称</th>
              <th>地址</th>
              <th class="text-right">条数</th>
              <th>上次更新</th>
              <th class="rt-col-actions">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="loading && rules.length === 0"><td :colspan="colSpan" class="empty">读取中…</td></tr>
            <tr v-else-if="rules.length === 0"><td :colspan="colSpan" class="empty">{{ empty }}</td></tr>
            <tr v-for="rule in rules" :key="`${rule.__tag || 'adguard'}:${rule.id || rule.name}`" :class="{ disabled: !rule.enabled }">
              <td class="rt-col-switch">
                <label class="switch switch-table">
                  <input type="checkbox" :checked="Boolean(rule.enabled)" :aria-label="`启用 ${rule.name}`" @change="toggle(rule)" />
                  <span class="slider"></span>
                </label>
              </td>
              <td :title="rule.name">{{ rule.name }}<span v-if="showType" class="muted rt-sub-type"> · {{ rule.__label }}</span></td>
              <td class="mono" :title="rule.url">{{ rule.url }}</td>
              <td class="text-right">{{ Number(rule.rule_count || 0).toLocaleString() }}</td>
              <td class="mono" :title="formatDateTime(rule.last_updated)">{{ formatDateTime(rule.last_updated) }}</td>
              <td class="rt-col-actions">
                <div class="rt-row-actions">
                  <button class="btn tiny warning" type="button" @click="updateOne(rule)">更新</button>
                  <button class="btn tiny secondary" type="button" @click="beginEdit(rule)">编辑</button>
                  <button class="btn tiny danger" type="button" @click="remove(rule)">删除</button>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>

    <Teleport to="body">
      <div v-if="editor.open" class="modal-mask" @click.self="editor.open = false">
        <section class="panel form-modal-card" role="dialog" :aria-label="editor.editing ? `编辑${title}` : `新增${title}`">
          <header class="panel-header">
            <h3>{{ editor.editing ? '编辑' : '新增' }}{{ title }}</h3>
            <button class="btn tiny secondary" type="button" aria-label="关闭" @click="editor.open = false">✕</button>
          </header>
          <div class="form-grid">
            <template v-if="showType">
              <label>类型</label>
              <select v-model="editor.type" :disabled="editor.readOnlyType">
                <option v-for="item in catalogs" :key="item.type" :value="item.type">{{ item.label }}</option>
              </select>
            </template>
            <label>地址</label>
            <input v-model="editor.url" placeholder="https://example.com/rules.txt" @input="onUrlInput" />
            <label>名称</label>
            <input v-model="editor.name" @input="touched.name = true" />
            <template v-if="!adguard">
              <label>本地文件</label>
              <input v-model="editor.files" placeholder="例如 srs/example.srs" @input="touched.files = true" />
            </template>
            <label>格式</label>
            <select v-model="editor.format" aria-label="订阅格式">
              <option v-for="item in SUBSCRIPTION_FORMATS" :key="item.value" :value="item.value">{{ item.label }}</option>
            </select>
            <template v-if="showCategory">
              <label>分类</label>
              <input v-model="editor.category" placeholder="例如 cn" aria-label="分类" />
            </template>
            <label>更新间隔（小时）</label>
            <input v-model.number="editor.update_interval_hours" type="number" min="1" />
            <label>启用</label>
            <label class="switch-inline"><input v-model="editor.enabled" type="checkbox" /><span>{{ editor.enabled ? '已启用' : '已禁用' }}</span></label>
            <label>自动更新</label>
            <label class="switch-inline"><input v-model="editor.auto_update" type="checkbox" /><span>{{ editor.auto_update ? '开启' : '关闭' }}</span></label>
            <template v-if="!adguard">
              <label>启用正则</label>
              <label class="switch-inline"><input v-model="editor.enable_regexp" type="checkbox" /><span>{{ editor.enable_regexp ? '开启' : '关闭' }}</span></label>
            </template>
          </div>
          <div class="actions">
            <button class="btn secondary" type="button" @click="editor.open = false">取消</button>
            <button class="btn primary" type="button" :disabled="editor.saving" @click="save">{{ editor.saving ? '保存中…' : '保存' }}</button>
          </div>
        </section>
      </div>
    </Teleport>
  </section>
</template>
