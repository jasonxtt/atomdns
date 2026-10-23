<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, getText, postJSON } from '../../api/http'
import { capabilityState, operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import CapabilityBoundary from '../../components/CapabilityBoundary.vue'
import SettingRow from '../../components/settings/SettingRow.vue'
import SettingSection from '../../components/settings/SettingSection.vue'
import { parseProcessMetrics } from '../../utils/overviewMetrics'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'

const update = reactive({
  loading: false,
  status: null
})
const processInfo = ref(parseProcessMetrics(''))
const now = ref(Date.now())

let restartProbeTimerId = 0

const health = computed(() => capabilityState.health || null)
// The host lists every directory under `<config root>/ui/` in its health (served at /<name>/).
const externalEntries = computed(() => (Array.isArray(health.value?.external_ui) ? health.value.external_ui : []))
// The directory this page was loaded from ('/' for the built-in UI, '/<name>/' for an external one).
const currentDirectory = typeof window !== 'undefined' ? new URL('./', window.location.href).pathname : '/'
const packageUrl = (entry) => `/${encodeURIComponent(entry.name)}/`
// Compatible UI packages other than the one being viewed.
const mountedPackages = computed(() => externalEntries.value.filter((entry) => entry.kind === 'mosdns_ui' && entry.mounted && packageUrl(entry) !== currentDirectory))
// Packages the host refused (incompatible, broken manifest, ...), with the reason.
const refusedPackages = computed(() => externalEntries.value.filter((entry) => entry.kind === 'mosdns_ui' && !entry.mounted))

function normalizeVersion(value) {
  if (!value) {
    return ''
  }
  return String(value).trim().toLowerCase().replace(/^v/, '')
}

const hasUpdate = computed(() => {
  const status = update.status
  if (!status || status.pending_restart) {
    return false
  }
  const cur = normalizeVersion(status.current_version)
  const latest = normalizeVersion(status.latest_version)
  if (cur && latest && cur === latest) {
    return false
  }
  return Boolean(status.update_available && status.download_url)
})

const updateBannerText = computed(() => {
  const status = update.status
  if (!status) {
    return '等待检查'
  }
  if (status.pending_restart) {
    return '更新已暂存，正在安全切换…'
  }
  if (status.message) {
    return String(status.message)
  }
  return hasUpdate.value ? '发现新版本，可立即更新。' : '当前已是最新版本'
})

function formatDate(value) {
  if (!value) {
    return '--'
  }
  const date = new Date(value)
  return Number.isFinite(date.getTime()) ? date.toLocaleString('zh-CN', { hour12: false }) : String(value)
}

const updateLastCheckedText = computed(() => formatDate(update.status?.checked_at))

function formatDuration(seconds) {
  if (!Number.isFinite(seconds) || seconds < 0) return '--'
  const days = Math.floor(seconds / 86400)
  const hours = Math.floor((seconds % 86400) / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  if (days > 0) return `${days} 天 ${hours} 小时`
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`
  return `${minutes} 分钟`
}

const uptimeText = computed(() => {
  const start = processInfo.value.startTime
  return start ? formatDuration(now.value / 1000 - start) : '--'
})

const resourceText = computed(() => {
  const info = processInfo.value
  const parts = []
  if (info.residentBytes !== null) parts.push(`内存 ${(info.residentBytes / 1024 / 1024).toFixed(1)} MiB`)
  if (info.cpuSeconds !== null) parts.push(`CPU 累计 ${info.cpuSeconds.toFixed(1)} 秒`)
  if (info.threads !== null) parts.push(`${info.threads} 线程`)
  if (info.openFds !== null) parts.push(`${info.openFds} 个打开的文件`)
  return parts.join(' · ') || '--'
})

const TRANSACTION_STATES = { idle: '空闲', pending: '更新进行中', rolled_back: '上次更新已回滚' }
const CONFIG_UPDATE_STATES = { success: '成功', failed: '失败', skipped: '跳过' }

// Configuration package facts: health (schemas, transaction) plus the update status
// (package state file and the last rollback).
const configPackage = computed(() => {
  const status = update.status || {}
  const applied = Number(health.value?.applied_config_schema ?? status.config_schema_applied ?? 0) || 0
  const required = Number(health.value?.required_config_schema ?? status.config_schema_required ?? 0) || 0
  const transaction = health.value?.update || null
  return {
    applied: applied ? `schema ${applied}` : '未记录',
    required: required ? `schema ${required}` : '--',
    current: applied > 0 && required > 0 && applied >= required,
    packageId: status.config_package_id || '',
    updateStatus: status.config_update_status ? (CONFIG_UPDATE_STATES[status.config_update_status] || status.config_update_status) : '',
    updateMessage: status.config_update_message || '',
    updateError: status.config_update_error || '',
    backup: status.config_update_backup || '',
    transactionState: transaction?.state ? (TRANSACTION_STATES[transaction.state] || transaction.state) : '',
    transactionTarget: transaction?.target_version || '',
    transactionError: transaction?.last_error || '',
    rollback: status.rollback_performed ? (status.rollback_message || '已回滚') : ''
  }
})

async function loadHealth() {
  if (!supportsOperation('system.health')) return
  await getJSON('/api/v1/system/health')
}

async function loadProcessInfo() {
  if (!supportsOperation('metrics.process') || !supportsOperation('metrics.cache')) return
  processInfo.value = parseProcessMetrics(await getText('/metrics'))
  now.value = Date.now()
}

async function loadUpdateStatus() {
  if (!supportsOperation('system.update')) return
  const status = await getJSON('/api/v1/update/status')
  update.status = status
  if (status?.config_auto_updated > 0) {
    setSuccess(`配置已自动更新（${status.config_auto_updated} 个文件）`)
  } else if (status?.config_update_status === 'failed') {
    setError(`配置自动升级失败: ${status.config_update_error || status.config_update_message || '未知错误'}`)
  }
}

async function checkUpdate() {
  if (!supportsOperation('system.update')) return
  clearTopNotice()
  update.loading = true
  try {
    update.status = await postJSON('/api/v1/update/check', {})
    setSuccess('已刷新最新版本信息')
  } catch (error) {
    setError(`检查更新失败: ${error.message}`)
  } finally {
    update.loading = false
  }
}

function stopRestartWatch() {
  if (restartProbeTimerId) {
    window.clearInterval(restartProbeTimerId)
    restartProbeTimerId = 0
  }
}

function startRestartWatch() {
  if (!supportsOperation('system.update')) return
  stopRestartWatch()
  const deadline = Date.now() + 90_000
  restartProbeTimerId = window.setInterval(async () => {
    if (Date.now() > deadline) {
      stopRestartWatch()
      return
    }
    try {
      const status = await getJSON('/api/v1/update/status')
      update.status = status
      if (!status?.pending_restart) {
        stopRestartWatch()
        setSuccess('重启完成')
        setTimeout(() => window.location.reload(), 800)
      }
    } catch {
      // ignore polling errors
    }
  }, 1000)
}

async function applyUpdate(force = false) {
  if (!supportsOperation('system.update')) return
  if (update.loading || (!force && !hasUpdate.value)) {
    return
  }
  clearTopNotice()
  update.loading = true
  try {
    const response = await postJSON('/api/v1/update/apply', { force, prefer_v3: false })
    const status = response?.status || response
    update.status = status
    setSuccess(String(response?.notes || status?.message || '更新请求已提交'))
    if (status?.pending_restart) {
      startRestartWatch()
    }
  } catch (error) {
    setError(`执行更新失败: ${error.message}`)
  } finally {
    update.loading = false
  }
}

async function reloadAll() {
  const results = await Promise.allSettled([loadHealth(), loadProcessInfo(), loadUpdateStatus()])
  const failure = results.find((result) => result.status === 'rejected')
  if (failure) {
    setError(`部分系统信息暂不可用: ${failure.reason?.message || failure.reason}`)
  }
}

onMounted(() => {
  reloadAll()
  window.addEventListener('mosdns-log-refresh', reloadAll)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', reloadAll)
  stopRestartWatch()
})
</script>

<template>
  <SettingSection id="update" title="版本与更新">
    <SettingRow label="程序版本" :description="processInfo.startTime ? `启动于 ${new Date(processInfo.startTime * 1000).toLocaleString('zh-CN', { hour12: false })}` : '当前运行的 rtdns'">
      <span class="rt-setting-text">{{ supportsOperation('system.version') ? (health?.version || '读取中') : operationReason('system.version') }}</span>
      <span v-if="supportsOperation('metrics.process')" class="muted rt-setting-note">已运行 {{ uptimeText }}</span>
    </SettingRow>
    <SettingRow label="资源占用" description="本进程当前的内存与累计 CPU 时间">
      <span class="rt-setting-text">{{ supportsOperation('metrics.process') ? resourceText : operationReason('metrics.process') }}</span>
    </SettingRow>
    <CapabilityBoundary operation="system.update">
      <SettingRow label="新版本" :description="`${updateBannerText}（上次检查 ${updateLastCheckedText}）`">
        <span class="rt-setting-text">{{ update.status?.latest_version || '--' }}</span>
        <button class="btn tiny secondary" :disabled="update.loading" @click="checkUpdate">{{ update.loading ? '处理中...' : '检查更新' }}</button>
      </SettingRow>
      <SettingRow label="安装更新" description="下载并校验新版本后原地切换；新版本起不来会自动回到旧版本">
        <button class="btn tiny primary" :disabled="update.loading || !hasUpdate" @click="applyUpdate(false)">立即更新</button>
        <button class="btn tiny danger" :disabled="update.loading || !update.status?.download_url" title="即使版本相同也重新安装" @click="applyUpdate(true)">强制更新</button>
      </SettingRow>
    </CapabilityBoundary>
    <SettingRow label="配置包" data-config-package>
      <template #description>
        程序需要的配置版本与本机已应用的版本<template v-if="configPackage.packageId">（{{ configPackage.packageId }}）</template>
        <span v-if="configPackage.updateMessage" class="rt-setting-subline">最近配置升级：{{ configPackage.updateStatus }} · {{ configPackage.updateMessage }}</span>
        <span v-if="configPackage.updateError" class="rt-setting-subline rt-text-bad">{{ configPackage.updateError }}</span>
        <span v-if="configPackage.backup" class="rt-setting-subline">升级前备份：<span class="mono">{{ configPackage.backup }}</span></span>
        <span v-if="configPackage.transactionError" class="rt-setting-subline rt-text-warn">最近一次回滚：{{ configPackage.transactionError }}</span>
        <span v-if="configPackage.rollback" class="rt-setting-subline rt-text-warn">{{ configPackage.rollback }}</span>
      </template>
      <span class="rt-setting-text">已应用 {{ configPackage.applied }} / 需要 {{ configPackage.required }}</span>
      <span v-if="configPackage.current" class="mini-badge">已是最新</span>
      <span v-if="configPackage.transactionState" class="muted rt-setting-note">更新事务：{{ configPackage.transactionState }}<template v-if="configPackage.transactionTarget"> · {{ configPackage.transactionTarget }}</template></span>
    </SettingRow>
    <SettingRow v-for="entry in mountedPackages" :key="`ui-${entry.name}`" label="外置界面" :description="`配置目录 ui/${entry.name} 下的界面包，版本 ${entry.ui_version}`" data-external-ui-link>
      <a class="btn tiny secondary" :href="packageUrl(entry)">前往 {{ packageUrl(entry) }}</a>
    </SettingRow>
    <SettingRow v-for="entry in refusedPackages" :key="`refused-${entry.name}`" label="外置界面" :description="`ui/${entry.name} 未挂载`" data-external-ui-rejected>
      <span class="rt-setting-text">{{ entry.reason }}</span>
    </SettingRow>
  </SettingSection>
</template>
