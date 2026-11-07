<script setup>
import { reactive, ref } from 'vue'
import { deleteRequest, postJSON, putJSON } from '../../api/http'
import { openConfirm } from '../../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'
import { formatDateTime } from '../../utils/time'
import { validateSubscription } from '../../utils/singbox'
import AppIcon from '../AppIcon.vue'

const props = defineProps({ subscriptions: { type: Array, default: () => [] }, groups: { type: Array, default: () => [] } })
const emit = defineEmits(['changed'])
const busy = ref('')
const updating = ref('')
const editor = reactive({ open: false, saving: false, editing: false, name: '', url: '', update_interval_hours: 24, enabled: true, use_proxy: true })

function beginAdd() {
  Object.assign(editor, { open: true, saving: false, editing: false, name: '', url: '', update_interval_hours: 24, enabled: true, use_proxy: true })
}
function beginEdit(row) {
  Object.assign(editor, { open: true, saving: false, editing: true, name: row.name, url: row.url, update_interval_hours: row.update_interval_hours, enabled: Boolean(row.enabled), use_proxy: Boolean(row.use_proxy) })
}
async function save() {
  const reason = validateSubscription(editor)
  if (reason) return setError(reason)
  editor.saving = true
  try {
    const body = { name: editor.name.trim(), url: editor.url.trim(), update_interval_hours: Number(editor.update_interval_hours), enabled: Boolean(editor.enabled), use_proxy: Boolean(editor.use_proxy) }
    if (editor.editing) await putJSON(`/api/v1/singbox/subscriptions/${encodeURIComponent(editor.name)}`, body)
    else await postJSON('/api/v1/singbox/subscriptions', body)
    setSuccess('订阅已保存并应用')
    editor.open = false
    emit('changed')
  } catch (error) { setError(`保存订阅失败: ${error.message}`) } finally { editor.saving = false }
}
async function toggle(row) {
  busy.value = row.name
  try {
    await putJSON(`/api/v1/singbox/subscriptions/${encodeURIComponent(row.name)}`, { ...row, enabled: !row.enabled })
    emit('changed')
  } catch (error) { setError(`切换订阅失败: ${error.message}`) } finally { busy.value = '' }
}
async function update(row) {
  updating.value = row.name
  clearTopNotice()
  try {
    await postJSON(`/api/v1/singbox/subscriptions/${encodeURIComponent(row.name)}/update`, {})
    setSuccess(`「${row.name}」已更新`)
    emit('changed')
  } catch (error) { setError(`更新订阅失败: ${error.message}`) } finally { updating.value = '' }
}
async function remove(row) {
  if (!(await openConfirm(`删除订阅「${row.name}」及其节点？引用这些节点的规则也会随之失效。`, { tone: 'danger' }))) return
  busy.value = row.name
  try {
    await deleteRequest(`/api/v1/singbox/subscriptions/${encodeURIComponent(row.name)}`)
    setSuccess('订阅已删除，sing-box 配置已重载')
    emit('changed')
  } catch (error) { setError(`删除订阅失败: ${error.message}`) } finally { busy.value = '' }
}
</script>

<template>
  <section class="panel rt-subscriptions" data-singbox-subscriptions>
    <header class="rt-list-head">
      <div class="rt-list-title"><h3>订阅列表</h3><span class="muted rt-list-count">共 {{ subscriptions.length }} 条</span></div>
      <button class="btn primary" type="button" @click="beginAdd">新增订阅</button>
    </header>
    <p class="muted rt-list-hint">支持 sing-box 原生订阅、Clash、SIP008 和分享链接格式。更新下载及节点健康检查由 sing-box 执行。</p>
    <div class="table-wrap rt-compact-table rt-sub-wrap">
      <table class="rt-sub-table">
        <thead><tr><th class="rt-col-switch">启用</th><th>名称</th><th>地址</th><th class="text-right">节点</th><th>上次更新</th><th class="rt-col-actions">操作</th></tr></thead>
        <tbody>
          <tr v-if="subscriptions.length === 0"><td colspan="6" class="empty">暂无订阅，点击「新增订阅」开始。</td></tr>
          <tr v-for="row in subscriptions" :key="row.name" :class="{ disabled: !row.enabled }">
            <td class="rt-col-switch"><label class="switch switch-table"><input type="checkbox" :checked="Boolean(row.enabled)" :disabled="busy === row.name" :aria-label="`启用 ${row.name}`" @change="toggle(row)"><span class="slider"></span></label></td>
            <td :title="row.name">{{ row.name }}<span class="muted rt-sub-type"> · {{ row.use_proxy ? '经代理下载' : '直连下载' }}</span></td>
            <td class="mono" :title="row.url">{{ row.url }}</td>
            <td class="text-right">{{ row.node_count ?? '—' }}</td>
            <td class="mono" :title="formatDateTime(row.updated_at)">{{ formatDateTime(row.updated_at) }}</td>
            <td class="rt-col-actions"><div class="rt-row-actions">
              <button class="rt-row-icon" type="button" title="更新" :aria-label="`更新 ${row.name}`" :disabled="updating === row.name || !row.enabled" @click="update(row)"><AppIcon name="refresh" :size="16" /></button>
              <button class="rt-row-icon" type="button" title="编辑" :aria-label="`编辑 ${row.name}`" @click="beginEdit(row)"><AppIcon name="pencil" :size="16" /></button>
              <button class="rt-row-icon danger" type="button" title="删除" :aria-label="`删除 ${row.name}`" :disabled="busy === row.name" @click="remove(row)"><AppIcon name="trash" :size="16" /></button>
            </div></td>
          </tr>
        </tbody>
      </table>
    </div>
    <Teleport to="body"><div v-if="editor.open" class="modal-mask modal-mask-center" @click.self="editor.open = false">
      <section class="panel form-modal-card" role="dialog" :aria-label="editor.editing ? '编辑订阅' : '新增订阅'">
        <header class="panel-header"><h3>{{ editor.editing ? '编辑订阅' : '新增订阅' }}</h3><button class="rt-row-icon" type="button" aria-label="关闭" @click="editor.open = false"><AppIcon name="x" :size="18" /></button></header>
        <div class="form-grid">
          <label>订阅名称</label><input v-model="editor.name" :disabled="editor.editing" placeholder="例如 airport" />
          <label>订阅地址</label><input v-model="editor.url" placeholder="https://example.com/subscribe" />
          <label>更新间隔（小时）</label><input v-model.number="editor.update_interval_hours" type="number" min="1" max="8760" />
          <label>下载方式</label><select v-model="editor.use_proxy"><option :value="true">通过 sing-box 出口下载</option><option :value="false">直连下载</option></select>
          <label>启用</label><label class="switch-inline"><input v-model="editor.enabled" type="checkbox" /><span>{{ editor.enabled ? '启用此订阅及其节点' : '停用（保留订阅设置）' }}</span></label>
        </div>
        <div class="actions"><button class="btn secondary" type="button" @click="editor.open = false">取消</button><button class="btn primary" type="button" :disabled="editor.saving" @click="save">{{ editor.saving ? '保存中…' : '保存并应用' }}</button></div>
      </section>
    </div></Teleport>
  </section>
</template>
