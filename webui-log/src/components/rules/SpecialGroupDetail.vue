<script setup>
import { computed, onBeforeUnmount, reactive, ref, watch } from 'vue'
import { deleteRequest, postJSON } from '../../api/http'
import { isNativeUpstreamReadOnly } from '../../api/nativeManagement'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { openConfirm } from '../../utils/confirm'
import { upstreamAddressDisplay, upstreamGroupDisplay } from '../../utils/upstreamStats'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'
import { clearDirty, setDirty } from '../../utils/unsaved'
import AppIcon from '../AppIcon.vue'
import SettingRow from '../settings/SettingRow.vue'
import ListEditor from './ListEditor.vue'
import SubscriptionTable from './SubscriptionTable.vue'
import UpstreamEditorModal from './UpstreamEditorModal.vue'

// One special group in one place: name and port, its upstreams (its own or references to
// the ordinary groups), its manual list, the subscriptions that point at it, and delete.
const props = defineProps({
  // The group as `GET /api/v1/special-groups` shows it; null creates a new one.
  group: { type: Object, default: null },
  // Ordinary upstream groups that can be referenced (`GET /api/v1/upstream/sources`).
  sources: { type: Array, default: () => [] },
  capabilities: { type: Object, default: null },
  proxySwitch: { type: Boolean, default: false },
  globalSocks5: { type: String, default: '' }
})
const emit = defineEmits(['saved', 'deleted', 'close'])

const uid = `group:${Math.random().toString(36).slice(2)}`
const canManage = computed(() => supportsOperation('groups.manage'))
const canUpstreams = computed(() => supportsOperation('groups.upstreams'))
const isNew = computed(() => !props.group)
const saving = ref(false)

function draftOf(group) {
  return {
    slot: Number(group?.slot) || 0,
    name: String(group?.name || ''),
    listenPort: group?.listen_port ? String(group.listen_port) : '',
    customPortOnly: Boolean(group?.custom_port_only && group?.listen_port),
    upstreamSources: Array.isArray(group?.upstream_sources) ? group.upstream_sources.map((item) => ({ ...item })) : [],
    owned: Array.isArray(group?.owned_upstreams) ? group.owned_upstreams.map((item) => ({ ...item })) : []
  }
}
const form = reactive(draftOf(props.group))
// What the server holds (reactive, so the form is clean again right after a save).
const baseline = ref(JSON.stringify(draftOf(props.group)))
const dirty = computed(() => JSON.stringify(form) !== baseline.value)
watch(dirty, (value) => setDirty(uid, value), { immediate: true })
// A refreshed group (after a save or an automatic refresh) replaces the form only while it
// has no unsaved edits; the lists and subscriptions below keep their own drafts.
watch(() => props.group, (next) => {
  if (dirty.value || !next) return
  Object.assign(form, draftOf(next))
  baseline.value = JSON.stringify(draftOf(next))
})
onBeforeUnmount(() => clearDirty(uid))

watch(() => form.listenPort, (value) => {
  if (!String(value || '').trim()) form.customPortOnly = false
})

// ----- references to ordinary upstream groups ------------------------------------
const draft = reactive({ pluginTag: '', kind: 'group', upstreamTag: '' })
watch(() => draft.pluginTag, () => { draft.upstreamTag = '' })
watch(() => draft.kind, () => { draft.upstreamTag = '' })
const draftGroup = computed(() => props.sources.find((item) => item?.plugin_tag === draft.pluginTag) || null)
const draftUpstreams = computed(() => (Array.isArray(draftGroup.value?.upstreams) ? draftGroup.value.upstreams : []))

function sourceLabel(source) {
  const title = upstreamGroupDisplay(source?.plugin_tag, []).title
  return source?.kind === 'group' ? `${title}（整个组）` : `${title} / ${source?.upstream_tag || '-'}`
}

function sourceStatus(source) {
  const group = props.sources.find((item) => item?.plugin_tag === source?.plugin_tag)
  if (!group) return '来源上游组不存在'
  if (source?.kind === 'group') {
    const enabled = (group.upstreams || []).filter((item) => item?.enabled).length
    return enabled > 0 ? `${enabled} 个上游当前启用` : '来源组没有启用中的上游'
  }
  const entry = (group.upstreams || []).find((item) => item?.tag === source?.upstream_tag)
  if (!entry) return '来源上游不存在'
  return entry.enabled ? '当前启用' : '来源上游已关闭'
}

function addSource() {
  const pluginTag = draft.pluginTag.trim()
  const kind = draft.kind === 'upstream' ? 'upstream' : 'group'
  const upstreamTag = draft.upstreamTag.trim()
  if (!pluginTag) return setError('请选择要引用的上游组')
  if (kind === 'upstream' && !upstreamTag) return setError('请选择要引用的上游')
  if (form.upstreamSources.some((item) => item.kind === kind && item.plugin_tag === pluginTag && item.upstream_tag === (kind === 'upstream' ? upstreamTag : undefined))) return setError('这个引用已经添加')
  form.upstreamSources.push({ kind, plugin_tag: pluginTag, ...(kind === 'upstream' ? { upstream_tag: upstreamTag } : {}) })
  draft.upstreamTag = ''
}

// ----- the group's own upstreams ---------------------------------------------------
const editing = reactive({ open: false, index: -1 })

const ownedStatus = (item) => isNativeUpstreamReadOnly(item || {}, props.capabilities)

function openOwned(index) {
  if (index >= 0 && ownedStatus(form.owned[index]).readOnly) return setError(ownedStatus(form.owned[index]).reason || '这条上游包含当前版本不支持的配置，保持只读')
  Object.assign(editing, { open: true, index })
}

function saveOwned(entry) {
  const tag = entry.tag
  if (form.owned.some((item, index) => index !== editing.index && String(item?.tag || '').trim() === tag)) return setError(`本组上游标识重复：${tag}`)
  if (editing.index >= 0) form.owned[editing.index] = { ...entry, enabled: Boolean(form.owned[editing.index]?.enabled) }
  else form.owned.push({ ...entry, enabled: true })
  editing.open = false
}

// ----- save and delete -------------------------------------------------------------
function portOf() {
  const text = String(form.listenPort || '').trim()
  if (!text) return 0
  const port = Number(text)
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('监听端口必须在 1-65535 之间')
  if (port === 53) throw new Error('监听端口不能使用 53')
  return port
}

async function save() {
  if (!canManage.value) return setError(operationReason('groups.manage'))
  const name = form.name.trim()
  if (!name) return setError('专属分流组名称不能为空')
  let port
  try {
    port = portOf()
  } catch (error) {
    return setError(error.message)
  }
  clearTopNotice()
  saving.value = true
  // What this request submits; edits made while it is in flight stay unsaved.
  const submitted = JSON.stringify(form)
  try {
    const body = {
      slot: form.slot,
      name,
      listen_port: port,
      custom_port_only: port !== 0 && Boolean(form.customPortOnly),
      ...(canUpstreams.value ? { upstream_sources: form.upstreamSources.map((item) => ({ ...item })), upstreams: form.owned.map((item) => ({ ...item })) } : {})
    }
    const saved = await postJSON('/api/v1/special-groups', body)
    baseline.value = submitted
    setSuccess('专属分流组已保存')
    emit('saved', saved)
  } catch (error) {
    setError(`保存专属分流组失败: ${error.message}`)
  } finally {
    saving.value = false
  }
}

async function remove() {
  if (!canManage.value || !props.group) return
  const ok = await openConfirm(`确定删除专属分流组“${props.group.name}”吗？它的上游、手动名单、订阅和监听端口都会一起删除，此操作不可恢复。`, { tone: 'danger', confirmText: '删除' })
  if (!ok) return
  try {
    await deleteRequest(`/api/v1/special-groups/${props.group.slot}`)
    setSuccess('专属分流组已删除')
    emit('deleted')
  } catch (error) {
    setError(`删除专属分流组失败: ${error.message}`)
  }
}

const routeText = computed(() => {
  const group = props.group
  if (!group) return ''
  if (!group.upstream_active) return '当前不参与分流'
  if (!group.listen_port) return '53 端口生效'
  return group.custom_port_only ? '仅自定义端口生效' : '53 端口和自定义端口都生效'
})
const catalogs = computed(() => (props.group ? [{ type: props.group.key, tag: props.group.diversion_plugin_tag, label: props.group.name }] : []))
</script>

<template>
  <div class="rt-group-detail" :data-group-detail="group ? group.slot : 'new'">
    <section class="panel rt-setting-section rt-setting-tiles rt-setting-tiles-compact rt-setting-joined">
      <h3 class="rt-setting-section-title">{{ isNew ? '新建专属分流组' : `${group.name}` }}</h3>
      <p v-if="!isNew" class="muted rt-setting-note">{{ routeText }} · 生效 {{ group.effective_upstream_count || 0 }} 个上游</p>
      <p v-if="group?.upstream_warnings?.length" class="rt-setting-error" role="alert">
        <span v-for="warning in group.upstream_warnings" :key="warning" class="rt-block">{{ warning }}</span>
      </p>
      <p v-if="!canManage" class="muted rt-setting-note" role="note">{{ operationReason('groups.manage') }}</p>
      <div class="rt-setting-grid">
        <SettingRow label="组名" layout="form" description="用来在页面里认出这个组。">
          <input v-model="form.name" class="rt-setting-input" placeholder="例如：移动上游" aria-label="组名" :disabled="!canManage" />
        </SettingRow>
        <SettingRow label="监听端口" layout="form" description="给这个组单独开一个 DNS 端口；留空则只在 53 端口生效。">
          <input v-model="form.listenPort" class="rt-setting-input rt-setting-input-short" type="number" min="1" max="65535" placeholder="留空" aria-label="监听端口" :disabled="!canManage" />
        </SettingRow>
        <SettingRow label="仅自定义端口" description="开启后，这个组只在自定义端口生效；关闭时 53 端口和自定义端口都生效。">
          <label class="switch"><input v-model="form.customPortOnly" type="checkbox" aria-label="仅自定义端口生效" :disabled="!canManage || !String(form.listenPort || '').trim()" /><span class="slider"></span></label>
        </SettingRow>
      </div>
    </section>

    <section class="panel rt-setting-section rt-setting-tiles rt-setting-tiles-compact rt-setting-joined" data-group-upstreams>
      <h3 class="rt-setting-section-title">上游</h3>
      <p v-if="!canUpstreams" class="muted rt-setting-note" role="note">{{ operationReason('groups.upstreams') }}</p>
      <div v-else class="rt-setting-grid">
        <div class="rt-setting-row rt-group-cell">
          <strong>引用已有上游</strong>
          <p class="muted rt-list-hint">引用会实时跟随来源上游组的修改、禁用和删除。</p>
          <div class="rt-group-add">
            <select v-model="draft.pluginTag" class="rt-setting-input" aria-label="来源上游组">
              <option value="" disabled>选择上游组</option>
              <option v-for="item in sources" :key="item.plugin_tag" :value="item.plugin_tag">{{ upstreamGroupDisplay(item.plugin_tag, []).title }}</option>
            </select>
            <select v-model="draft.kind" class="rt-setting-input" aria-label="引用范围">
              <option value="group">整个上游组</option>
              <option value="upstream">组内单个上游</option>
            </select>
            <select v-if="draft.kind === 'upstream'" v-model="draft.upstreamTag" class="rt-setting-input" aria-label="来源上游">
              <option value="" disabled>选择上游</option>
              <option v-for="item in draftUpstreams" :key="item.tag" :value="item.tag">{{ item.tag || '未设置标识' }}{{ item.enabled ? '' : '（已关闭）' }}</option>
            </select>
            <button class="btn secondary" type="button" :disabled="!canManage" @click="addSource">添加引用</button>
          </div>
          <ul v-if="form.upstreamSources.length > 0" class="rt-link-list" data-group-sources>
            <li v-for="(source, index) in form.upstreamSources" :key="`${source.kind}-${source.plugin_tag}-${source.upstream_tag || 'group'}`">
              <span>{{ sourceLabel(source) }} <span class="muted">{{ sourceStatus(source) }}</span></span>
              <button class="rt-row-icon" type="button" title="解绑" :aria-label="`解绑 ${sourceLabel(source)}`" :disabled="!canManage" @click="form.upstreamSources.splice(index, 1)"><AppIcon name="x" :size="16" /></button>
            </li>
          </ul>
          <p v-else class="muted rt-list-hint">暂无引用</p>
        </div>
        <div class="rt-setting-row rt-group-cell">
          <div class="rt-list-head">
            <strong>本组专属上游</strong>
            <button class="btn tiny primary" type="button" :disabled="!canManage" @click="openOwned(-1)">添加上游</button>
          </div>
          <p class="muted rt-list-hint">只属于这个组，删除组时一起删除。</p>
          <ul v-if="form.owned.length > 0" class="rt-link-list" data-group-owned>
            <li v-for="(item, index) in form.owned" :key="`${item.tag}-${index}`">
              <span>{{ item.tag || '未设置标识' }} <span class="muted">{{ item.protocol || '-' }} · {{ upstreamAddressDisplay(item) }} · {{ item.enabled ? '已启用' : '已关闭' }}<template v-if="ownedStatus(item).readOnly"> · 只读</template></span></span>
              <span class="rt-upstream-group-actions">
                <button class="rt-row-icon" type="button" :disabled="!canManage || ownedStatus(item).readOnly" :title="ownedStatus(item).reason || '编辑'" :aria-label="`编辑 ${item.tag || '上游'}`" @click="openOwned(index)"><AppIcon name="pencil" :size="16" /></button>
                <button class="rt-row-icon danger" type="button" title="删除" :aria-label="`删除 ${item.tag || '上游'}`" :disabled="!canManage" @click="form.owned.splice(index, 1)"><AppIcon name="trash" :size="16" /></button>
              </span>
            </li>
          </ul>
          <p v-else class="muted rt-list-hint">暂无专属上游</p>
        </div>
      </div>
    </section>

    <div class="actions rt-group-actions">
      <button class="btn primary" type="button" :disabled="saving || !canManage || (!isNew && !dirty)" data-group-save @click="save">{{ saving ? '保存中…' : isNew ? '创建专属分流组' : '保存修改' }}</button>
      <button class="btn secondary" type="button" @click="emit('close')">返回组列表</button>
      <button v-if="!isNew" class="btn danger" type="button" :disabled="!canManage" data-group-delete @click="remove">删除这个组</button>
    </div>
    <p v-if="dirty && !isNew" class="muted rt-list-hint">名称、端口和上游有未保存的改动。</p>

    <template v-if="!isNew">
      <ListEditor :tag="group.manual_plugin_tag" :title="`${group.name}的手动名单`" hint="这里的域名会直接归入这个组，并使用这个组的上游和缓存。支持 full:、domain:、keyword:、regexp: 等写法。" />
      <SubscriptionTable kind="online" title="指向这个组的订阅" :catalogs="catalogs" hint="订阅里的域名都归入这个组，并使用这个组的上游。" />
    </template>
    <p v-else class="muted rt-list-hint">创建后可以为这个组设置手动名单和订阅。</p>

    <UpstreamEditorModal
      v-if="editing.open"
      :group-title="form.name || '本组'"
      :entry="editing.index >= 0 ? form.owned[editing.index] : null"
      :capabilities="capabilities"
      :global-socks5="globalSocks5"
      :proxy-switch="proxySwitch"
      @close="editing.open = false"
      @save="saveOwned"
    />
  </div>
</template>
