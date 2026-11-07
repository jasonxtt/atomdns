<script setup>
import { computed, reactive, ref, watch } from 'vue'
import { deleteRequest, postJSON, putJSON } from '../../api/http'
import { openConfirm } from '../../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'
import { latestDelay, validateNode } from '../../utils/singbox'
import AppIcon from '../AppIcon.vue'

const props = defineProps({ groups: { type: Array, default: () => [] }, manualNodes: { type: Array, default: () => [] }, outbounds: { type: Array, default: () => [] } })
const emit = defineEmits(['changed'])
const selectedGroup = ref('')
const selectedNode = ref('')
const testing = ref('')
const delays = reactive({})
const busy = ref('')
const editor = reactive({ open: false, saving: false, editing: false, tag: '', link: '', enabled: true })
const group = computed(() => props.groups.find((item) => item.name === selectedGroup.value) || props.groups[0] || null)
watch(() => props.groups, (groups) => {
  if (!groups.some((item) => item.name === selectedGroup.value)) selectedGroup.value = groups[0]?.name || ''
  selectedNode.value = group.value?.now || ''
}, { immediate: true })
const nodes = computed(() => group.value?.nodes || [])

async function select() {
  if (!group.value || !selectedNode.value || selectedNode.value === group.value.now) return
  busy.value = group.value.name
  try {
    await postJSON('/api/v1/singbox/select', { group: group.value.name, node: selectedNode.value })
    setSuccess(`「${group.value.name}」已切换到 ${selectedNode.value}`)
    emit('changed')
  } catch (error) { setError(`切换出口失败: ${error.message}`) } finally { busy.value = '' }
}
async function delay(node) {
  testing.value = node.name
  try {
    const result = await postJSON('/api/v1/singbox/delay', { name: node.name, timeout: 5000 })
    if (result?.error) throw new Error(result.error)
    delays[node.name] = result?.delay
    setSuccess(`${node.name}: ${result.delay} ms`)
  } catch (error) { setError(`${node.name} 测速失败: ${error.message}`) } finally { testing.value = '' }
}
function beginAdd() {
  Object.assign(editor, { open: true, saving: false, editing: false, tag: '', link: '', enabled: true })
}
function beginEdit(node) {
  const saved = props.manualNodes.find((item) => item.tag === node.name) || { tag: node.name, enabled: true }
  Object.assign(editor, { open: true, saving: false, editing: true, tag: saved.tag, link: '', enabled: saved.enabled })
}
async function save() {
  const reason = validateNode(editor)
  if (reason) return setError(reason)
  editor.saving = true
  try {
    const body = { tag: editor.tag.trim(), link: editor.link.trim(), enabled: Boolean(editor.enabled) }
    if (editor.editing) await putJSON(`/api/v1/singbox/nodes/${encodeURIComponent(editor.tag)}`, body)
    else await postJSON('/api/v1/singbox/nodes', body)
    setSuccess('节点已保存，sing-box 配置已重载')
    editor.open = false
    emit('changed')
  } catch (error) { setError(`保存节点失败: ${error.message}`) } finally { editor.saving = false }
}
async function remove(node) {
  if (!(await openConfirm(`删除手动节点「${node.tag}」？绑定它的域名规则将找不到出口并被阻断。`, { tone: 'danger' }))) return
  busy.value = node.tag
  try {
    await deleteRequest(`/api/v1/singbox/nodes/${encodeURIComponent(node.tag)}`)
    setSuccess('手动节点已删除')
    emit('changed')
  } catch (error) { setError(`删除节点失败: ${error.message}`) } finally { busy.value = '' }
}
</script>

<template>
  <section class="panel rt-subscriptions" data-singbox-nodes>
    <header class="rt-list-head">
      <div class="rt-list-title"><h3>节点与出口</h3><span class="muted rt-list-count">{{ nodes.length }} 个节点</span></div>
      <button class="btn primary" type="button" @click="beginAdd">添加节点</button>
    </header>
    <div v-if="groups.length > 1" class="rt-singbox-group-tabs" role="tablist" aria-label="出口组">
      <button v-for="item in groups" :key="item.name" type="button" class="rt-subtab" :class="{ active: group?.name === item.name }" @click="selectedGroup = item.name">{{ item.name }}</button>
    </div>
    <template v-if="group">
      <div class="rt-singbox-selector">
        <label for="singbox-node-select">当前出口</label>
        <select id="singbox-node-select" v-model="selectedNode" :disabled="busy === group.name">
          <option v-for="node in nodes" :key="node.name" :value="node.name">{{ node.name }}{{ node.type ? ` (${node.type})` : '' }}</option>
        </select>
        <button class="btn primary" type="button" :disabled="!selectedNode || selectedNode === group.now || busy === group.name" @click="select">应用出口</button>
        <span class="muted">当前：{{ group.now || '未选择' }}</span>
      </div>
      <div class="table-wrap rt-compact-table rt-sub-wrap">
        <table class="rt-sub-table">
          <thead><tr><th>节点</th><th>类型</th><th>来源</th><th>延迟</th><th class="rt-col-actions">操作</th></tr></thead>
          <tbody>
            <tr v-if="nodes.length === 0"><td colspan="5" class="empty">此出口组暂无节点，请先添加订阅或节点。</td></tr>
            <tr v-for="node in nodes" :key="node.name" :class="{ 'rt-singbox-selected': node.selected }">
              <td :title="node.name">{{ node.name }}<span v-if="node.selected" class="rt-flag-mark">使用中</span></td>
              <td>{{ node.type }}</td><td>{{ node.source || '手动节点' }}</td>
              <td class="mono">{{ delays[node.name] ? `${delays[node.name]} ms` : latestDelay(node) }}</td>
              <td class="rt-col-actions"><div class="rt-row-actions">
                <button class="rt-row-icon" type="button" title="测速" :aria-label="`测速 ${node.name}`" :disabled="testing === node.name" @click="delay(node)"><AppIcon name="gauge" :size="16" /></button>
                <template v-if="!node.source">
                  <button class="rt-row-icon" type="button" title="编辑" :aria-label="`编辑 ${node.name}`" @click="beginEdit(node)"><AppIcon name="pencil" :size="16" /></button>
                  <button class="rt-row-icon danger" type="button" title="删除" :aria-label="`删除 ${node.name}`" :disabled="busy === node.name" @click="remove(node)"><AppIcon name="trash" :size="16" /></button>
                </template>
              </div></td>
            </tr>
          </tbody>
        </table>
      </div>
    </template>
    <p v-else class="muted rt-list-hint">还没有出口组。添加一个节点或订阅后即可在这里切换出口。</p>
    <p class="muted rt-list-hint">手动节点通过节点链接导入，支持 socks5h://、vless:// 等 sing-box 支持的协议。订阅节点只读，请到「订阅管理」更新或删除。</p>
    <Teleport to="body"><div v-if="editor.open" class="modal-mask modal-mask-center" @click.self="editor.open = false">
      <section class="panel form-modal-card" role="dialog" :aria-label="editor.editing ? '编辑节点' : '添加节点'">
        <header class="panel-header"><h3>{{ editor.editing ? '编辑节点' : '添加节点' }}</h3><button class="rt-row-icon" type="button" aria-label="关闭" @click="editor.open = false"><AppIcon name="x" :size="18" /></button></header>
        <div class="form-grid">
          <label for="singbox-node-link">{{ editor.editing ? '替换节点链接（留空保持不变）' : '节点链接' }}</label><textarea id="singbox-node-link" v-model="editor.link" rows="5" required placeholder="socks5h://example.com:1080#节点名称
vless://…"></textarea>
          <label for="singbox-node-tag">节点标识（可选）</label><input id="singbox-node-tag" v-model="editor.tag" :disabled="editor.editing" placeholder="默认使用链接 # 后的名称" />
          <p class="muted rt-list-hint">仅粘贴一条链接；认证和协议参数保存在 sing-box 节点配置中。编辑时留空可保留原节点。</p>
          <label>启用</label><label class="switch-inline"><input v-model="editor.enabled" type="checkbox" /><span>{{ editor.enabled ? '启用' : '停用（保留配置）' }}</span></label>
        </div>
        <div class="actions"><button class="btn secondary" type="button" @click="editor.open = false">取消</button><button class="btn primary" type="button" :disabled="editor.saving" @click="save">{{ editor.saving ? '保存中…' : '保存并应用' }}</button></div>
      </section>
    </div></Teleport>
  </section>
</template>
