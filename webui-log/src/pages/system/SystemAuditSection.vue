<script setup>
import { onBeforeUnmount, onMounted, reactive } from 'vue'
import { getJSON, postJSON } from '../../api/http'
import { capabilityFetch, supportsOperation } from '../../api/runtimeCapabilities'
import CapabilityBoundary from '../../components/CapabilityBoundary.vue'
import SettingRow from '../../components/settings/SettingRow.vue'
import SettingSection from '../../components/settings/SettingSection.vue'
import { openConfirm } from '../../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'

const audit = reactive({
  capturing: null,
  capacity: null,
  newCapacity: '',
  busy: false,
  error: ''
})

async function postEmpty(url) {
  const response = await capabilityFetch(url, { method: 'POST' })
  if (!response.ok) {
    let message = `HTTP ${response.status} ${response.statusText}`
    try {
      const data = await response.json()
      if (data?.error) message = data.error
    } catch {
      const text = await response.text().catch(() => '')
      if (text) message = text
    }
    throw new Error(message)
  }
  return response
}

async function loadAuditStatusAndCapacity() {
  if (!supportsOperation('audit.read') && !supportsOperation('audit.capacity')) return
  audit.error = ''
  const [statusResult, capacityResult] = await Promise.allSettled([
    supportsOperation('audit.read') ? getJSON('/api/v1/audit/status') : Promise.resolve({ capturing: null }),
    supportsOperation('audit.capacity') ? getJSON('/api/v1/audit/capacity') : Promise.resolve({ capacity: null })
  ])
  if (supportsOperation('audit.read') && statusResult.status === 'fulfilled') {
    audit.capturing = Boolean(statusResult.value?.capturing)
  }
  if (supportsOperation('audit.capacity') && capacityResult.status === 'fulfilled') {
    audit.capacity = Number(capacityResult.value?.capacity || 0)
  }
  const failure = [statusResult, capacityResult].find((result) => result.status === 'rejected')
  if (failure) {
    audit.error = `审计状态读取失败: ${failure.reason?.message || failure.reason}`
  }
}

async function toggleAuditCapture() {
  if (!supportsOperation('audit.control')) return
  if (audit.busy || audit.capturing === null) {
    return
  }
  clearTopNotice()
  audit.busy = true
  audit.error = ''
  try {
    if (audit.capturing) {
      await postEmpty('/api/v1/audit/stop')
      setSuccess('审计已停止')
    } else {
      await postEmpty('/api/v1/audit/start')
      setSuccess('审计已启动')
    }
    await loadAuditStatusAndCapacity()
  } catch (error) {
    setError(`切换审计状态失败: ${error.message}`)
    audit.error = `最近一次操作失败: ${error.message}`
  } finally {
    audit.busy = false
  }
}

async function clearAuditLogs() {
  if (!supportsOperation('audit.control')) return
  if (audit.busy || audit.capturing === null) {
    return
  }
  if (!(await openConfirm('将删除已保存的全部查询详情，概览与日志页的统计会从零开始，不可恢复。', { tone: 'danger' }))) {
    return
  }
  clearTopNotice()
  audit.busy = true
  audit.error = ''
  try {
    await postEmpty('/api/v1/audit/clear')
    setSuccess('日志已清空')
    await loadAuditStatusAndCapacity()
  } catch (error) {
    setError(`清空日志失败: ${error.message}`)
    audit.error = `最近一次操作失败: ${error.message}`
  } finally {
    audit.busy = false
  }
}

async function submitCapacity() {
  if (!supportsOperation('audit.capacity')) return
  if (audit.busy || audit.capacity === null) {
    return
  }
  const rawCapacity = String(audit.newCapacity ?? '').trim()
  const capacity = Number(rawCapacity)
  if (!rawCapacity || !Number.isInteger(capacity) || capacity < 0 || capacity > 400000) {
    setError('请输入 0 到 400000 之间的整数')
    return
  }
  if (!(await openConfirm(`将最多保存 ${capacity.toLocaleString()} 条查询详情。修改上限会清空已保存的详细日志，确认继续？`, { tone: 'danger' }))) {
    return
  }
  clearTopNotice()
  audit.busy = true
  audit.error = ''
  try {
    await postJSON('/api/v1/audit/capacity', { capacity })
    audit.newCapacity = ''
    setSuccess(`详细日志上限已设为 ${capacity.toLocaleString()} 条`)
    await loadAuditStatusAndCapacity()
  } catch (error) {
    setError(`设置详细日志上限失败: ${error.message}`)
    audit.error = `最近一次操作失败: ${error.message}`
  } finally {
    audit.busy = false
  }
}

onMounted(() => {
  loadAuditStatusAndCapacity()
  window.addEventListener('atomdns-log-refresh', loadAuditStatusAndCapacity)
})

onBeforeUnmount(() => {
  window.removeEventListener('atomdns-log-refresh', loadAuditStatusAndCapacity)
})
</script>

<template>
  <SettingSection id="audit" title="审计日志" tiles>
    <CapabilityBoundary operation="audit.control">
      <SettingRow label="查询记录" layout="form" description="记录每条查询的详情，概览与日志页的统计都来自这里；停止后不再新增">
        <span class="rt-setting-text" :class="audit.capturing ? 'rt-text-ok' : ''">{{ audit.capturing === null ? '读取中...' : (audit.capturing ? '记录中' : '已停止') }}</span>
        <button class="btn tiny primary" :disabled="audit.busy || audit.capturing === null" @click="toggleAuditCapture">{{ audit.busy ? '处理中...' : (audit.capturing ? '停止' : '开始') }}</button>
        <button class="btn tiny danger" :disabled="audit.busy || audit.capturing === null" @click="clearAuditLogs">清空</button>
      </SettingRow>
    </CapabilityBoundary>
    <CapabilityBoundary operation="audit.capacity">
      <SettingRow label="详细日志上限" layout="form" description="最多保存多少条查询详情（0–400000，0 表示不保存）。修改上限会清空已保存的详细日志">
        <span class="rt-setting-text">{{ audit.capacity === null ? '读取中...' : `${Number(audit.capacity).toLocaleString()} 条` }}</span>
        <form class="rt-setting-inline" @submit.prevent="submitCapacity">
          <input v-model="audit.newCapacity" class="rt-setting-input rt-setting-input-short" :disabled="audit.busy || audit.capacity === null" type="number" min="0" max="400000" step="1" placeholder="新上限" aria-label="新的详细日志上限" />
          <button class="btn tiny primary" :disabled="audit.busy || audit.capacity === null" type="submit">设置</button>
        </form>
      </SettingRow>
    </CapabilityBoundary>
    <p v-if="audit.error" class="muted rt-text-warn rt-setting-error">{{ audit.error }}</p>
  </SettingSection>
</template>
