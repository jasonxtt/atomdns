<script setup>
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, postJSON } from '../../api/http'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import CapabilityBoundary from '../../components/CapabilityBoundary.vue'
import SettingRow from '../../components/settings/SettingRow.vue'
import SettingSection from '../../components/settings/SettingSection.vue'
import { openConfirm } from '../../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'

const restarting = ref(false)
const resetting = ref(false)
const webuiPort = reactive({
  loading: false,
  saving: false,
  input: '',
  activePort: 0,
  activeAddr: ''
})

async function requestRestart(delayMs) {
  await postJSON('/api/v1/system/restart', { delay_ms: delayMs })
}

async function restartHost() {
  if (!supportsOperation('system.restart')) return
  if (!(await openConfirm('确认重启 AtomDNS？', { tone: 'danger' }))) {
    return
  }
  clearTopNotice()
  restarting.value = true
  try {
    await requestRestart(500)
    setSuccess('已发送重启请求，页面将自动刷新。')
    setTimeout(() => window.location.reload(), 4000)
  } catch (error) {
    setError(`重启失败: ${error.message}`)
  } finally {
    restarting.value = false
  }
}

// The overview statistics live in the process (audit ring, counters): a restart clears them.
async function resetStatistics() {
  if (!supportsOperation('system.restart') || resetting.value) return
  const confirmed = await openConfirm('将清空概览页全部统计并重启 AtomDNS，是否继续？', {
    title: '重置统计',
    confirmText: '继续',
    tone: 'danger'
  })
  if (!confirmed) {
    return
  }
  resetting.value = true
  try {
    await requestRestart(500)
    setSuccess('已发送重启请求，页面将自动刷新。')
    window.setTimeout(() => window.location.reload(), 4000)
  } catch (error) {
    setError(`重置统计失败: ${error.message}`)
  } finally {
    resetting.value = false
  }
}

function buildWebUIRootUrl(port) {
  try {
    const url = new URL(window.location.href)
    url.port = String(port)
    // An external UI (served at /<name>/) stays in its own directory after the port changes:
    // its resource URLs are relative, so the page's own directory is the right target.
    url.pathname = new URL('./', window.location.href).pathname
    url.search = ''
    url.hash = ''
    return url.toString()
  } catch {
    return `${window.location.protocol}//${window.location.hostname}:${port}/`
  }
}

async function loadWebUIPortSettings() {
  if (!supportsOperation('system.webui_port')) return
  webuiPort.loading = true
  try {
    const payload = await getJSON('/api/v1/system/webui-port')
    const port = Number(payload?.port || 0)
    webuiPort.activePort = Number(payload?.active_port || 0)
    webuiPort.activeAddr = String(payload?.active_addr || '')
    webuiPort.input = port > 0 ? String(port) : ''
  } catch (error) {
    setError(`读取 WebUI 端口失败: ${error.message}`)
  } finally {
    webuiPort.loading = false
  }
}

async function applyWebUIPortAndRestart() {
  if (!supportsOperation('system.webui_port')) return
  const port = Number.parseInt(String(webuiPort.input || '').trim(), 10)
  if (!Number.isFinite(port) || port < 1 || port > 65535) {
    setError('请输入 1-65535 之间的端口')
    return
  }
  if (!(await openConfirm(`将 WebUI 端口改为 ${port} 并重启 AtomDNS，确认继续？`, { tone: 'danger' }))) {
    return
  }
  clearTopNotice()
  webuiPort.saving = true
  try {
    await postJSON('/api/v1/system/webui-port', { port })
    await requestRestart(800)
    const target = buildWebUIRootUrl(port)
    setSuccess(`端口已更新为 ${port}，AtomDNS 正在重启，稍后将跳转到新地址。`)
    setTimeout(() => {
      window.location.href = target
    }, 4500)
  } catch (error) {
    setError(`设置 WebUI 端口失败: ${error.message}`)
    await loadWebUIPortSettings()
  } finally {
    webuiPort.saving = false
  }
}

onMounted(() => {
  loadWebUIPortSettings()
  window.addEventListener('atomdns-log-refresh', loadWebUIPortSettings)
})

onBeforeUnmount(() => {
  window.removeEventListener('atomdns-log-refresh', loadWebUIPortSettings)
})
</script>

<template>
  <SettingSection id="service" title="服务" tiles>
    <CapabilityBoundary operation="system.webui_port">
      <SettingRow label="WebUI 端口" layout="form" :description="webuiPort.activeAddr ? `当前监听 ${webuiPort.activeAddr}。修改后 AtomDNS 会重启并跳转到新端口` : '修改后 AtomDNS 会重启并跳转到新端口'">
        <input v-model="webuiPort.input" class="rt-setting-input rt-setting-input-short" type="number" min="1" max="65535" placeholder="端口" aria-label="WebUI 端口" />
        <button class="btn tiny primary" :disabled="webuiPort.saving || webuiPort.loading" @click="applyWebUIPortAndRestart">{{ webuiPort.saving ? '处理中...' : '保存并重启' }}</button>
      </SettingRow>
    </CapabilityBoundary>
    <SettingRow label="重启与重置" layout="form" description="原地重启（进程号不变），期间 DNS 短暂不可用。重置统计会清空概览页的全部统计，做法就是重启" data-service-restart>
      <span v-if="!supportsOperation('system.restart')" class="muted rt-setting-note" role="note">{{ operationReason('system.restart') }}</span>
      <button class="btn tiny secondary" :disabled="restarting || !supportsOperation('system.restart')" @click="restartHost">{{ restarting ? '处理中...' : '重启 AtomDNS' }}</button>
      <button class="btn tiny danger" :disabled="resetting || !supportsOperation('system.restart')" title="清空概览页全部统计并重启 AtomDNS" @click="resetStatistics">{{ resetting ? '重置中...' : '重置统计' }}</button>
    </SettingRow>
  </SettingSection>
</template>
