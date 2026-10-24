<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, postJSON } from '../../api/http'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { openConfirm } from '../../utils/confirm'
import { setError, setSuccess } from '../../utils/notice'
import { formatRelativeTime, isZeroTime } from '../../utils/time'
import SettingRow from '../settings/SettingRow.vue'

// requery: refreshes the cache and warms it up again from the learned and ranked names.
// Shows its state, starts or cancels a run and edits the timer.
const available = ref(true)
const loadError = ref('')
const status = ref(null)
const config = ref(null)
const action = ref('')
const saving = ref(false)
const form = reactive({ enabled: false, intervalMinutes: 0, dateRangeDays: 30, startDatetimeLocal: '' })
let pollTimer = 0
let saveTimer = 0

const running = computed(() => String(status.value?.task_state || '') === 'running')
const stateText = computed(() => {
  switch (String(status.value?.task_state || 'idle')) {
    case 'running': return '正在执行'
    case 'failed': return '上次执行失败'
    case 'cancelled': return '上次已取消'
    default: return '空闲'
  }
})
const progress = computed(() => {
  const processed = Number(status.value?.progress?.processed || 0)
  const total = Number(status.value?.progress?.total || 0)
  return { processed, total, percent: total > 0 ? Math.max(0, Math.min(100, (processed / total) * 100)) : 0 }
})
const lastRun = computed(() => {
  const value = status.value
  if (!value?.last_run_start_time || isZeroTime(value.last_run_start_time)) return '从未执行'
  if (value.last_run_end_time && !isZeroTime(value.last_run_end_time)) {
    const seconds = Math.max(0, (new Date(value.last_run_end_time) - new Date(value.last_run_start_time)) / 1000)
    return `完成于 ${formatRelativeTime(value.last_run_end_time)}（耗时 ${seconds < 1 ? '不到 1' : Math.round(seconds)} 秒）`
  }
  return `开始于 ${formatRelativeTime(value.last_run_start_time)}`
})
const lastError = computed(() => (String(status.value?.task_state || '') === 'failed' ? String(status.value?.last_error || '').trim() : ''))
const lastCount = computed(() => {
  const count = Number(status.value?.last_run_domain_count)
  return Number.isFinite(count) && count >= 0 ? `${count.toLocaleString()} 条` : '-'
})

function toLocalInput(value) {
  if (!value || isZeroTime(value)) return ''
  const date = new Date(value)
  if (!Number.isFinite(date.getTime())) return ''
  const pad = (number) => String(number).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

function applyConfig() {
  const scheduler = config.value?.scheduler || {}
  form.enabled = Boolean(scheduler.enabled)
  form.intervalMinutes = Number(scheduler.interval_minutes || 0)
  form.dateRangeDays = Number(config.value?.execution_settings?.date_range_days || 30)
  form.startDatetimeLocal = toLocalInput(scheduler.start_datetime)
}

function poll() {
  window.clearInterval(pollTimer)
  pollTimer = 0
  if (running.value) pollTimer = window.setInterval(load, 3000)
}

async function load() {
  if (!supportsOperation('cache.requery')) return
  try {
    const [nextStatus, nextConfig] = await Promise.all([getJSON('/plugins/requery/status'), getJSON('/plugins/requery')])
    available.value = true
    loadError.value = ''
    status.value = nextStatus
    // The form follows the server unless a save is waiting.
    if (!saveTimer) {
      config.value = nextConfig
      applyConfig()
    }
  } catch (error) {
    if (error.status === 404) {
      available.value = false
      status.value = null
      loadError.value = '当前配置没有 requery 插件，已跳过这一部分。'
    } else {
      loadError.value = `读取状态失败: ${error.message}`
    }
  }
  poll()
}

async function trigger() {
  if (!(await openConfirm('将启动一次全新的刷新任务，完整执行所有步骤，可能需要一些时间。'))) return
  action.value = 'trigger'
  try {
    await postJSON('/plugins/requery/trigger', {})
    setSuccess('刷新任务已开始')
    await load()
  } catch (error) {
    setError(`开始任务失败: ${error.message}`)
  } finally {
    action.value = ''
  }
}

async function cancel() {
  if (!(await openConfirm('确定要取消正在执行的刷新任务吗？', { tone: 'danger' }))) return
  action.value = 'cancel'
  try {
    await postJSON('/plugins/requery/cancel', {})
    setSuccess('已发送取消请求')
    await load()
  } catch (error) {
    setError(`取消任务失败: ${error.message}`)
  } finally {
    action.value = ''
  }
}

async function saveScheduler() {
  window.clearTimeout(saveTimer)
  saveTimer = 0
  // The timer form starts from what the plugin holds; without that, a save would write defaults.
  if (!config.value) return setError('定时设置还没有成功读取，请先刷新')
  const interval = Number(form.intervalMinutes || 0)
  const days = Number(form.dateRangeDays || 0)
  if (form.enabled && interval <= 0) return setError('启用定时任务时，必须设置有效的间隔分钟数')
  if (days <= 0) return setError('域名刷新天数必须大于 0')
  saving.value = true
  try {
    const start = form.startDatetimeLocal ? new Date(form.startDatetimeLocal) : null
    if (start && !Number.isFinite(start.getTime())) throw new Error('首次执行时间格式无效')
    await postJSON('/plugins/requery/scheduler/config', {
      enabled: Boolean(form.enabled),
      interval_minutes: interval,
      start_datetime: start ? start.toISOString() : '',
      date_range_days: days
    })
    setSuccess('定时设置已保存')
    await load()
  } catch (error) {
    setError(`保存定时设置失败: ${error.message}`)
  } finally {
    saving.value = false
  }
}

// Typing in a field saves a moment after the last change.
function saveSoon() {
  window.clearTimeout(saveTimer)
  saveTimer = window.setTimeout(saveScheduler, 1500)
}

onMounted(() => {
  load()
  window.addEventListener('mosdns-log-refresh', load)
})
onBeforeUnmount(() => {
  window.clearInterval(pollTimer)
  window.clearTimeout(saveTimer)
  window.removeEventListener('mosdns-log-refresh', load)
})
</script>

<template>
  <section class="panel rt-setting-section" data-requery>
    <h3 class="rt-setting-section-title">刷新缓存与预热</h3>
    <p v-if="!supportsOperation('cache.requery')" class="muted rt-setting-note" role="note">{{ operationReason('cache.requery') }}</p>
    <p v-else-if="!available" class="muted rt-setting-note" role="note">{{ loadError }}</p>
    <template v-else>
      <p v-if="loadError" class="rt-setting-error" role="alert">{{ loadError }}</p>
      <SettingRow label="当前状态" description="清掉缓存并按学到的和排行靠前的域名重新查询，让缓存重新热起来。">
        <span class="rt-flag-mark" :class="{ 'rt-flag-running': running }" data-requery-state>{{ stateText }}</span>
        <button v-if="!running" class="btn primary" type="button" :disabled="action === 'trigger'" @click="trigger">{{ action === 'trigger' ? '启动中…' : '立即运行' }}</button>
        <button v-else class="btn danger" type="button" :disabled="action === 'cancel'" @click="cancel">{{ action === 'cancel' ? '取消中…' : '取消任务' }}</button>
      </SettingRow>
      <SettingRow label="上次运行" description="最近一次任务的结果。">
        <span class="rt-setting-text">{{ lastRun }}</span>
        <span class="rt-setting-text muted">刷新 {{ lastCount }}</span>
      </SettingRow>
      <p v-if="lastError" class="rt-setting-error" role="alert">最近失败原因：{{ lastError }}</p>
      <div v-if="running" class="requery-progress-bar" role="progressbar" :aria-valuenow="Math.floor(progress.percent)">
        <div class="requery-progress-bar-fill" :style="{ width: `${progress.percent}%` }"></div>
        <span class="requery-progress-text">{{ Math.floor(progress.percent) }}%（{{ progress.processed.toLocaleString() }} / {{ progress.total.toLocaleString() }}）</span>
      </div>
      <SettingRow label="定时运行" description="按间隔自动运行；修改后自动保存。">
        <label class="switch"><input v-model="form.enabled" type="checkbox" aria-label="启用定时运行" @change="saveScheduler" /><span class="slider"></span></label>
      </SettingRow>
      <SettingRow label="首次执行时间" description="定时运行从这个时间开始。">
        <input v-model="form.startDatetimeLocal" class="rt-setting-input" type="datetime-local" :disabled="!form.enabled" aria-label="首次执行时间" @change="saveSoon" />
      </SettingRow>
      <SettingRow label="间隔（分钟）" description="两次定时运行之间的间隔。">
        <input v-model.number="form.intervalMinutes" class="rt-setting-input rt-setting-input-short" type="number" min="1" :disabled="!form.enabled" aria-label="间隔分钟" @change="saveSoon" />
      </SettingRow>
      <SettingRow label="刷新天数" description="只刷新最近这么多天内出现过的域名。">
        <input v-model.number="form.dateRangeDays" class="rt-setting-input rt-setting-input-short" type="number" min="1" aria-label="域名刷新天数" @change="saveSoon" />
        <span class="rt-setting-note">{{ saving ? '保存中…' : '' }}</span>
      </SettingRow>
    </template>
  </section>
</template>
