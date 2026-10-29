<script setup>
import AppIcon from './AppIcon.vue'
import CapabilityBoundary from './CapabilityBoundary.vue'
import DetailField from './DetailField.vue'
import { supportsOperation, operationReason } from '../api/runtimeCapabilities'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getJSON, postJSON } from '../api/http'
import { normalizeAliasMap, normalizeIP } from '../utils/clientAliases'
import { latencyTone } from '../utils/overviewMetrics'
import { clearTopNotice, setError, setSuccess } from '../utils/notice'
import { formatDateTime, formatRelativeTime as formatRelativeTimeText } from '../utils/time'

const props = defineProps({
  mode: {
    type: String,
    default: 'live'
  }
})

defineEmits(['open-diagnostic', 'close'])

const loading = ref(false)

const searchInput = ref('')
const logs = ref([])
const pagination = ref({
  total_items: 0,
  total_pages: 0,
  current_page: 1,
  items_per_page: 50
})
const selectedLog = ref(null)
const detailModalOpen = ref(false)

const clientAliases = ref({})
const specialGroups = ref([])

const captureDuration = ref(15)
const captureLogs = ref([])
const diagnosticRequests = ref([])
const selectedDiagnosticKey = ref('__raw__')
const showFalseLogs = ref(false)

const isLiveMode = computed(() => props.mode === 'live')

const activeDiagnosticRequest = computed(() => {
  if (selectedDiagnosticKey.value === '__raw__') {
    return null
  }
  return diagnosticRequests.value.find((item) => item.key === selectedDiagnosticKey.value) || null
})

const structuredItems = computed(() => {
  const request = activeDiagnosticRequest.value
  if (!request || !Array.isArray(request.logs) || request.logs.length === 0) {
    return []
  }
  const startAt = new Date(request.logs[0].time).getTime()
  let lastLogger = ''
  const items = []

  request.logs.forEach((log, index) => {
    const logger = String(log.logger_name || '')
    const msg = String(log.msg || '')
    if (logger && logger !== lastLogger && !msg.includes('lazy cache')) {
      items.push({
        kind: 'section',
        key: `section-${logger}-${index}`,
        text: `进入序列: ${logger}`
      })
      lastLogger = logger
    }

    let icon = ''
    let label = ''
    let resultText = ''
    let isFalse = false
    if (String(log.matcher_name || '')) {
      icon = '?'
      label = String(log.matcher_name)
      const matched = parseMatchResult(log.match_result)
      if (matched === true) {
        resultText = 'TRUE'
      } else if (matched === false) {
        resultText = 'FALSE'
        isFalse = true
      } else {
        resultText = '—'
      }
    } else if (String(log.plugin_name || '')) {
      icon = 'P'
      label = String(log.plugin_name)
    } else if (msg) {
      icon = 'L'
      label = msg
    }

    if (!label) {
      return
    }

    const currentAt = new Date(log.time).getTime()
    const diffMs = Number.isFinite(currentAt) && Number.isFinite(startAt) ? Math.max(0, currentAt - startAt) : 0
    items.push({
      kind: 'item',
      key: `item-${index}`,
      icon,
      label,
      resultText,
      isFalse,
      timeDiffMs: diffMs
    })
  })

  return items
})

const flowDurationMs = computed(() => {
  const request = activeDiagnosticRequest.value
  if (!request || !Array.isArray(request.logs) || request.logs.length === 0) {
    return 0
  }
  const startAt = new Date(request.logs[0].time).getTime()
  const endAt = new Date(request.logs[request.logs.length - 1].time).getTime()
  if (!Number.isFinite(startAt) || !Number.isFinite(endAt)) {
    return 0
  }
  return Math.max(0, endAt - startAt)
})

const responseFlagText = computed(() => {
  const flags = selectedLog.value?.response_flags || {}
  const items = []
  if (flags.RA ?? flags.ra) {
    items.push('RA')
  }
  if (flags.AA ?? flags.aa) {
    items.push('AA')
  }
  if (flags.TC ?? flags.tc) {
    items.push('TC')
  }
  return items.length ? items.join(', ') : '-'
})

const detailActionFields = computed(() => {
  const log = selectedLog.value
  if (!log) {
    return {}
  }
  return {
    client_ip: {
      key: 'client_ip',
      label: '客户端',
      value: getClientLabel(log.client_ip),
      copyValue: normalizeIP(log.client_ip),
      filterValue: normalizeIP(log.client_ip),
      exact: true,
      mono: false
    },
    query_name: {
      key: 'query_name',
      label: '域名',
      value: log.query_name || '-',
      copyValue: String(log.query_name || '').trim(),
      filterValue: String(log.query_name || '').trim(),
      exact: false,
      mono: false
    },
    domain_set: {
      key: 'domain_set',
      label: '分流规则',
      value: log.domain_set || '-',
      copyValue: String(log.domain_set || '').trim(),
      filterValue: String(log.domain_set || '').trim(),
      exact: true,
      mono: false
    },
    trace_id: {
      key: 'trace_id',
      label: 'Trace ID',
      value: log.trace_id || '-',
      copyValue: String(log.trace_id || '').trim(),
      filterValue: String(log.trace_id || '').trim(),
      exact: true,
      mono: true
    }
  }
})

function getDisplayName(ip) {
  const normalizedIp = normalizeIP(ip)
  return clientAliases.value[normalizedIp] || normalizedIp || '-'
}

function hasAlias(ip) {
  const normalizedIp = normalizeIP(ip)
  return Boolean(normalizedIp && clientAliases.value[normalizedIp] && clientAliases.value[normalizedIp] !== normalizedIp)
}

function getClientLabel(ip) {
  const normalizedIp = normalizeIP(ip)
  if (!normalizedIp) {
    return '-'
  }
  const alias = clientAliases.value[normalizedIp]
  return alias ? `${alias} (${normalizedIp})` : normalizedIp
}

function getIpMatchesByAlias(alias, exact = false) {
  const searchTerm = String(alias || '').trim().toLowerCase()
  if (!searchTerm) {
    return []
  }
  const entries = Object.entries(clientAliases.value)
  const matches = []
  for (let index = 0; index < entries.length; index += 1) {
    const [ip, name] = entries[index]
    const aliasName = String(name || '').trim().toLowerCase()
    if (!aliasName) {
      continue
    }
    const isMatch = exact ? aliasName === searchTerm : aliasName.includes(searchTerm)
    if (isMatch) {
      matches.push(ip)
    }
  }
  return [...new Set(matches)]
}

function getMatchedGroupDisplay(value) {
  const key = String(value || '').trim()
  if (!key) {
    return '-'
  }
  const hit = specialGroups.value.find((item) => String(item?.key || '') === key)
  return hit?.name || key
}

function resetMessages() {
  clearTopNotice()
}

function parseSearchKeyword(raw) {
  const term = String(raw || '').trim()
  if (!term) {
    return { query: '', exact: false }
  }
  if (term.startsWith('"') && term.endsWith('"') && term.length >= 2) {
    return { query: term.slice(1, -1), exact: true }
  }
  return { query: term, exact: false }
}

function formatTime(value) {
  return formatDateTime(value)
}

function formatLogRelativeTime(value) {
  return formatRelativeTimeText(value, { olderFormat: 'date' })
}

function responseSummary(log) {
  if (!log) {
    return '-'
  }
  if (log.response_code && log.response_code !== 'NOERROR') {
    return log.response_code
  }
  const answers = Array.isArray(log.answers) ? log.answers : []
  if (answers.length === 0) {
    return '(empty)'
  }
  const first = answers.find((item) => item.type === 'A' || item.type === 'AAAA' || item.type === 'CNAME') || answers[0]
  let text = String(first?.data || '')
  if (text.length > 28) {
    text = `${text.slice(0, 25)}...`
  }
  if (answers.length > 1) {
    text += ` (+${answers.length - 1})`
  }
  return text
}

function logRowKey(item, index) {
  return `${item.trace_id || 'trace'}-${item.query_time || 'time'}-${item.client_ip || 'client'}-${index}`
}

// Only the slow ones get a colour; a normal duration keeps the text colour.
function durationClass(ms) {
  const tone = latencyTone(Number(ms))
  return tone === 'warn' || tone === 'bad' ? `rt-text-${tone}` : ''
}

function openLogDetail(item) {
  selectedLog.value = item || null
  detailModalOpen.value = Boolean(item)
}

function closeLogDetail() {
  detailModalOpen.value = false
}

async function copyText(value) {
  const text = String(value || '').trim()
  if (!text) {
    setError('没有可复制的内容')
    return
  }
  try {
    if (navigator.clipboard?.writeText && window.isSecureContext) {
      await navigator.clipboard.writeText(text)
    } else {
      const textArea = document.createElement('textarea')
      textArea.value = text
      textArea.setAttribute('readonly', 'readonly')
      textArea.style.position = 'fixed'
      textArea.style.left = '-9999px'
      document.body.appendChild(textArea)
      textArea.select()
      document.execCommand('copy')
      document.body.removeChild(textArea)
    }
    setSuccess('已复制到剪贴板')
  } catch (error) {
    setError(`复制失败: ${error.message}`)
  }
}

function applyQuickFilter(value, exact = false) {
  const text = String(value || '').trim()
  if (!text) {
    setError('没有可筛选的内容')
    return
  }
  searchInput.value = exact ? `"${text}"` : text
  closeLogDetail()
  refreshLogs()
}

function handleOpenLogFilter(event) {
  if (!isLiveMode.value) {
    return
  }
  const detail = event?.detail || {}
  const text = String(detail.value || '').trim()
  if (!text) {
    return
  }
  searchInput.value = detail.exact ? `"${text}"` : text
  refreshLogs()
}

function getDetailActionField(key) {
  return detailActionFields.value?.[key] || null
}

function parseMatchResult(value) {
  if (typeof value === 'boolean') {
    return value
  }
  if (typeof value === 'number') {
    if (value === 1) {
      return true
    }
    if (value === 0) {
      return false
    }
  }
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase()
    if (normalized === 'true') {
      return true
    }
    if (normalized === 'false') {
      return false
    }
  }
  return null
}

function normalizeCaptureLog(item) {
  if (!item || typeof item !== 'object') {
    return null
  }
  return {
    time: item.time || item.timestamp || '',
    msg: item.msg || item.message || '',
    logger_name: item.logger_name || item.logger || '',
    trace_id: item.trace_id || item.traceId || '',
    domain: item.domain || item.query_name || item.queryName || '',
    matcher_name: item.matcher_name || item.matcher || '',
    plugin_name: item.plugin_name || item.plugin || '',
    match_result: item.match_result ?? item.matched ?? item.result,
    level: item.level || ''
  }
}

function sortByTimeAsc(list) {
  return [...list].sort((a, b) => new Date(a.time).getTime() - new Date(b.time).getTime())
}

function processDiagnosticLogs(allLogs) {
  const normalized = allLogs.map(normalizeCaptureLog).filter(Boolean)
  captureLogs.value = sortByTimeAsc(normalized)

  const requestMap = new Map()
  captureLogs.value.forEach((log) => {
    if (!log.trace_id) {
      return
    }
    if (!requestMap.has(log.trace_id)) {
      requestMap.set(log.trace_id, {
        key: log.trace_id,
        traceId: log.trace_id,
        domain: log.domain || 'N/A',
        startTime: log.time,
        logs: []
      })
    }
    const request = requestMap.get(log.trace_id)
    if (log.domain && request.domain === 'N/A') {
      request.domain = log.domain
    }
    request.logs.push(log)
  })

  const requests = [...requestMap.values()].map((request) => {
    request.logs = sortByTimeAsc(request.logs)
    request.startTime = request.logs[0]?.time || request.startTime
    return request
  })
  requests.sort((a, b) => new Date(b.startTime).getTime() - new Date(a.startTime).getTime())

  diagnosticRequests.value = requests
  selectedDiagnosticKey.value = '__raw__'
  showFalseLogs.value = false
}

async function loadAliases() {
  if (!supportsOperation('client.aliases')) return
  try {
    const data = await getJSON('/plugins/clientname')
    clientAliases.value = normalizeAliasMap(data)
  } catch {
    clientAliases.value = {}
  }
}

async function loadSpecialGroups() {
  if (!supportsOperation('groups.read')) return
  try {
    const data = await getJSON('/api/v1/special-groups')
    specialGroups.value = Array.isArray(data) ? data : []
  } catch {
    specialGroups.value = []
  }
}

async function loadLogs(page = 1, append = false) {
  if (!supportsOperation('audit.read')) return
  loading.value = true
  resetMessages()
  const { query: rawQuery, exact } = parseSearchKeyword(searchInput.value)
  let query = rawQuery
  const aliasIPs = query ? getIpMatchesByAlias(query, exact) : []
  if (aliasIPs.length > 0) {
    query = ''
  }
  try {
    const params = new URLSearchParams({
      page: String(page),
      limit: '50'
    })
    aliasIPs.forEach((ip) => {
      params.append('client_ip', ip)
    })
    if (query) {
      params.set('q', query)
      params.set('exact', String(exact))
    }
    const data = await getJSON(`/api/v2/audit/logs?${params.toString()}`)
    const nextLogs = Array.isArray(data?.logs) ? data.logs : []
    pagination.value = data?.pagination || pagination.value
    logs.value = append ? [...logs.value, ...nextLogs] : nextLogs
    if (!append) {
      selectedLog.value = null
      detailModalOpen.value = false
    }
  } catch (error) {
    setError(`加载查询日志失败: ${error.message}`)
  } finally {
    loading.value = false
  }
}

function refreshLogs() {
  return loadLogs(1, false)
}

function loadMoreLogs() {
  const page = Number(pagination.value.current_page || 1)
  const totalPages = Number(pagination.value.total_pages || 1)
  if (loading.value || page >= totalPages) {
    return
  }
  return loadLogs(page + 1, true)
}

async function startCapture() {
  if (!supportsOperation('capture.logs')) return
  const seconds = Math.max(1, Math.min(600, Number(captureDuration.value || 15)))
  loading.value = true
  resetMessages()
  try {
    await postJSON('/api/v1/capture/start', { duration_seconds: seconds })
    setSuccess(`已开始抓取，时长 ${seconds} 秒`)
  } catch (error) {
    setError(`启动抓取失败: ${error.message}`)
  } finally {
    loading.value = false
  }
}

async function fetchCaptureLogs() {
  if (!supportsOperation('capture.logs')) return
  loading.value = true
  resetMessages()
  try {
    const data = await getJSON('/api/v1/capture/logs')
    const list = Array.isArray(data) ? data : Array.isArray(data?.logs) ? data.logs : Array.isArray(data?.items) ? data.items : []
    processDiagnosticLogs(list)
    if (captureLogs.value.length === 0) {
      setError('没有采集到任何 Debug 日志')
      return
    }
    setSuccess(`成功获取 ${captureLogs.value.length} 条日志`)
  } catch (error) {
    setError(`获取抓取日志失败: ${error.message}`)
  } finally {
    loading.value = false
  }
}

function renderLogType(log) {
  if (log.matcher_name) {
    return 'Matcher'
  }
  if (log.plugin_name) {
    return 'Plugin'
  }
  return 'Log'
}

function renderLogName(log) {
  return log.matcher_name || log.plugin_name || log.logger_name || log.msg || ''
}

function handleGlobalRefresh() {
  if (isLiveMode.value) {
    refreshLogs()
    return
  }
  fetchCaptureLogs()
}

onMounted(async () => {
  // Registered first: a filter handed over from the overview arrives right after mounting.
  window.addEventListener('mosdns-log-refresh', handleGlobalRefresh)
  window.addEventListener('mosdns-open-log-filter-ready', handleOpenLogFilter)
  await Promise.all([
    loadAliases(),
    loadSpecialGroups()
  ])
  if (isLiveMode.value) {
    refreshLogs()
  }
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', handleGlobalRefresh)
  window.removeEventListener('mosdns-open-log-filter-ready', handleOpenLogFilter)
})
</script>

<template>
  <section class="sub-panel">
    <div v-if="isLiveMode" class="panel">
      <header class="panel-header">
        <p class="muted">支持分页与关键字过滤。输入带双引号表示精确匹配；支持按客户端别名搜索。</p>
        <div class="actions">
          <button class="btn secondary" type="button" data-open-diagnostic @click="$emit('open-diagnostic')">诊断抓取</button>
          <CapabilityBoundary operation="client.aliases"><router-link class="btn secondary" to="/rules/clients" data-open-aliases>客户端别名</router-link></CapabilityBoundary>
        </div>
      </header>

      <form class="query-search" @submit.prevent="refreshLogs">
        <label class="query-search-field">
          <AppIcon name="search" :size="16" />
          <input v-model="searchInput" placeholder="全局搜索，使用 &quot;引号&quot; 精确匹配" aria-label="搜索日志" />
        </label>
        <button class="btn primary" type="submit" :disabled="loading">搜索</button>
        <button class="btn secondary" type="button" :disabled="loading" @click="searchInput = ''; refreshLogs()">清空</button>
      </form>

      <p class="muted">
        当前第 {{ pagination.current_page || 1 }} / {{ pagination.total_pages || 1 }} 页，
        共 {{ Number(pagination.total_items || 0).toLocaleString() }} 条
      </p>

      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>时间</th>
              <th>域名 / 响应</th>
              <th>类型</th>
              <th class="text-right">耗时 (ms)</th>
              <th>客户端</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="loading && logs.length === 0">
              <td colspan="5" class="empty">加载中...</td>
            </tr>
            <tr v-else-if="logs.length === 0">
              <td colspan="5" class="empty">暂无日志</td>
            </tr>
            <tr
              v-for="(item, index) in logs"
              :key="logRowKey(item, index)"
              class="log-row"
              :class="{ selected: selectedLog === item }"
              @click="openLogDetail(item)"
            >
              <td class="log-time">{{ formatLogRelativeTime(item.query_time) }}</td>
              <td>
                <div>
                  <span class="status-dot" :class="!item.response_code || item.response_code === 'NOERROR' ? 'ok' : 'fail'"></span>
                  <strong>{{ item.query_name }}</strong>
                </div>
                <div class="muted">{{ responseSummary(item) }}</div>
              </td>
              <td><span class="type-chip">{{ item.query_type || '-' }}</span></td>
              <td class="text-right log-duration" :class="durationClass(item.duration_ms)">{{ Number(item.duration_ms || 0).toFixed(2) }}</td>
              <td>
                <div><strong>{{ getDisplayName(item.client_ip) }}</strong></div>
                <div v-if="hasAlias(item.client_ip)" class="muted mono">{{ normalizeIP(item.client_ip) }}</div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="actions log-more">
        <button
          class="btn secondary"
          :disabled="loading || Number(pagination.current_page || 1) >= Number(pagination.total_pages || 1)"
          @click="loadMoreLogs"
        >
          加载更多
        </button>
      </div>
    </div>

    <div v-else class="panel">
      <header class="panel-header">
        <p class="muted">开启一段时间的调试抓取，再获取日志查看每条查询经过的匹配器与插件。</p>
        <div class="actions">
          <button class="btn secondary" type="button" data-back-to-live @click="$emit('close')">返回实时查询</button>
        </div>
      </header>

      <CapabilityBoundary operation="capture.logs"><div class="diagnostic-toolbar">
        <label>抓取时长(秒)</label>
        <input v-model.number="captureDuration" type="number" min="1" max="600" />
        <button class="btn secondary" :disabled="loading" @click="startCapture">{{ loading ? '处理中...' : '日志抓取' }}</button>
        <button class="btn primary" :disabled="loading" @click="fetchCaptureLogs">{{ loading ? '处理中...' : '获取日志' }}</button>
      </div>

      </CapabilityBoundary>
      <div class="diagnostic-layout">
        <section class="panel sub-panel diagnostic-pane">
          <h4>请求列表</h4>
          <div class="diagnostic-request-list">
            <button
              class="diagnostic-request-item"
              :class="{ active: selectedDiagnosticKey === '__raw__' }"
              @click="selectedDiagnosticKey = '__raw__'"
            >
              <span class="domain">原始日志</span>
              <span class="details">全部 {{ captureLogs.length }} 条</span>
            </button>

            <button
              v-for="item in diagnosticRequests"
              :key="item.key"
              class="diagnostic-request-item"
              :class="{ active: selectedDiagnosticKey === item.key }"
              @click="selectedDiagnosticKey = item.key"
            >
              <span class="domain">{{ item.domain || 'N/A' }}</span>
              <span class="details">{{ formatTime(item.startTime) }} · {{ item.traceId }}</span>
            </button>
          </div>
        </section>

        <section class="panel sub-panel diagnostic-pane">
          <h4>分析结果</h4>

          <div v-if="selectedDiagnosticKey === '__raw__'" class="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>时间</th>
                  <th>Trace ID</th>
                  <th>类型</th>
                  <th>名称/信息</th>
                  <th>结果</th>
                  <th>域名</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="captureLogs.length === 0">
                  <td colspan="6" class="empty">暂无抓取数据</td>
                </tr>
                <tr v-for="(log, index) in captureLogs" :key="`raw-${index}`">
                  <td>{{ formatTime(log.time) }}</td>
                  <td class="mono">{{ log.trace_id || '-' }}</td>
                  <td>{{ renderLogType(log) }}</td>
                  <td>{{ renderLogName(log) }}</td>
                  <td>
                    <span v-if="parseMatchResult(log.match_result) === true" class="result-badge ok">TRUE</span>
                    <span v-else-if="parseMatchResult(log.match_result) === false" class="result-badge fail">FALSE</span>
                    <span v-else>-</span>
                  </td>
                  <td>{{ log.domain || '-' }}</td>
                </tr>
              </tbody>
            </table>
          </div>

          <div v-else-if="activeDiagnosticRequest" class="structured-flow">
            <header class="timeline-header">
              <h5>
                DNS 查询流程: <strong>{{ activeDiagnosticRequest.domain || 'N/A' }}</strong>
                (总耗时: {{ flowDurationMs }} ms)
              </h5>
              <button class="btn tiny secondary" @click="showFalseLogs = !showFalseLogs">
                {{ showFalseLogs ? '隐藏 FALSE' : '显示 FALSE' }}
              </button>
            </header>

            <div class="structured-log-list">
              <template v-for="item in structuredItems" :key="item.key">
                <div v-if="item.kind === 'section'" class="log-section-header">{{ item.text }}</div>
                <div
                  v-else-if="showFalseLogs || !item.isFalse"
                  class="log-item"
                  :class="{ 'result-fail': item.isFalse }"
                >
                  <div class="log-item-main">
                    <span class="icon">{{ item.icon }}</span>
                    <span class="content">{{ item.label }}</span>
                  </div>
                  <div class="log-item-meta">
                    <span v-if="item.resultText && item.resultText !== '—'" class="result-badge" :class="{ ok: item.resultText === 'TRUE', fail: item.resultText === 'FALSE' }">
                      {{ item.resultText }}
                    </span>
                    <span class="time">+{{ item.timeDiffMs }}ms</span>
                  </div>
                </div>
              </template>
            </div>
          </div>

          <p v-else class="muted">点击左侧请求查看分流流程。</p>
        </section>
      </div>
    </div>

    <Teleport to="body">
      <div v-if="detailModalOpen && selectedLog" class="modal-mask modal-mask-center" @click.self="closeLogDetail">
        <section class="panel data-view-modal detail-modal" role="dialog" aria-label="查询详情">
          <header class="panel-header">
            <div>
              <h3>查询详情</h3>
              <p class="muted">{{ selectedLog.query_name || '-' }} · {{ formatTime(selectedLog.query_time) }}</p>
            </div>
            <button class="rt-row-icon" type="button" aria-label="关闭" title="关闭" @click="closeLogDetail"><AppIcon name="x" :size="18" /></button>
          </header>
          <div class="detail-scroll">
            <h4 class="detail-group-title">基本信息</h4>
            <div class="detail-fields">
              <DetailField label="时间" :value="formatTime(selectedLog.query_time)" />
              <DetailField label="客户端" :field="getDetailActionField('client_ip')" @copy="copyText" @filter="applyQuickFilter" />
              <DetailField label="域名" :field="getDetailActionField('query_name')" @copy="copyText" @filter="applyQuickFilter" />
              <DetailField label="类型" :value="selectedLog.query_type || '-'" />
              <DetailField label="类别" :value="selectedLog.query_class || '-'" />
              <DetailField label="Trace ID" :field="getDetailActionField('trace_id')" wide @copy="copyText" @filter="applyQuickFilter" />
            </div>

            <div class="rt-divider" role="separator"><i></i></div>
            <h4 class="detail-group-title">响应</h4>
            <div class="detail-fields">
              <DetailField label="响应码" :value="`${selectedLog.response_code || '-'}${selectedLog.is_blocked ? ' (已拦截)' : ''}`" />
              <DetailField label="响应标志" :value="responseFlagText" />
              <DetailField label="应答详情" :value="`${selectedLog.answer_details_status || 'complete'}${selectedLog.answer_decode_error ? `（${selectedLog.answer_decode_error}）` : ''}`" />
              <DetailField label="耗时" :value="`${Number(selectedLog.duration_ms || 0).toFixed(2)} ms`" />
            </div>
            <div class="table-wrap detail-answers">
              <table>
                <thead>
                  <tr>
                    <th>应答类型</th>
                    <th>数据</th>
                    <th>TTL</th>
                  </tr>
                </thead>
                <tbody>
                  <tr v-if="!Array.isArray(selectedLog.answers) || selectedLog.answers.length === 0">
                    <td colspan="3" class="empty">(empty)</td>
                  </tr>
                  <tr v-for="(answer, index) in (selectedLog.answers || [])" :key="`answer-${index}`">
                    <td>{{ answer.type || '-' }}</td>
                    <td class="mono">{{ answer.data || '-' }}</td>
                    <td>{{ Number(answer.ttl || 0) }}s</td>
                  </tr>
                </tbody>
              </table>
            </div>

            <div class="rt-divider" role="separator"><i></i></div>
            <h4 class="detail-group-title">分流与上游</h4>
            <div class="detail-fields">
              <DetailField label="生效标签" :value="selectedLog.effective_tag || selectedLog.domain_set || '-'" />
              <DetailField label="命中标签" :value="selectedLog.domain_set || '-'" />
              <DetailField label="匹配来源" :value="selectedLog.matched_rule_source || '-'" />
              <DetailField label="分流规则" :field="getDetailActionField('domain_set')" @copy="copyText" @filter="applyQuickFilter" />
              <DetailField label="专属分流组" :value="getMatchedGroupDisplay(selectedLog.matched_group)" />
              <DetailField label="最终序列" :value="selectedLog.final_sequence || '-'" />
              <DetailField label="最终上游组" :value="selectedLog.final_upstream || '-'" />
              <DetailField label="上游目标" :value="selectedLog.upstream_targets || '-'" />
              <DetailField label="最终上游" :value="selectedLog.selected_upstream || '-'" />
            </div>
            <p v-if="selectedLog.upstream_diagnostics && ![1, 2].includes(selectedLog.upstream_diagnostics.schema_version)" class="muted">
              原生上游诊断版本暂不支持（{{ selectedLog.upstream_diagnostics.schema_version ?? '-' }}）；普通响应与路由信息仍可用。
            </p>
            <div v-if="[1, 2].includes(selectedLog.upstream_diagnostics?.schema_version)" class="detail-diagnostics">
              <span class="detail-label">原生上游诊断</span>
              <div v-if="selectedLog.upstream_diagnostics.schema_version === 2 && selectedLog.upstream_diagnostics.branches?.length" class="table-wrap upstream-diagnostics-table">
                <table>
                  <thead><tr><th>ID</th><th>父级</th><th>角色</th><th>策略</th><th>QTYPE</th><th>决定</th></tr></thead>
                  <tbody>
                    <tr v-for="branch in selectedLog.upstream_diagnostics.branches" :key="`branch-${branch.id}`">
                      <td>{{ branch.id ?? '-' }}</td>
                      <td>{{ branch.parent_id ?? '-' }}</td>
                      <td>{{ branch.role || '-' }}</td>
                      <td class="mono">{{ branch.policy || '-' }}</td>
                      <td>{{ branch.qtype ?? '-' }}</td>
                      <td>{{ branch.decision || '-' }}</td>
                    </tr>
                  </tbody>
                </table>
              </div>
              <p class="detail-selected">
                <template v-if="selectedLog.upstream_diagnostics.selected">
                  选中 {{ selectedLog.upstream_diagnostics.selected.entry || '-' }} ·
                  {{ selectedLog.upstream_diagnostics.selected.peer || '-' }} ·
                  {{ selectedLog.upstream_diagnostics.selected.transport || '-' }}
                </template>
                <template v-else>未选中最终网络上游</template>
              </p>
              <div class="table-wrap upstream-diagnostics-table">
                <table>
                  <thead><tr><th>序号</th><th v-if="selectedLog.upstream_diagnostics.schema_version === 2">分支</th><th v-if="selectedLog.upstream_diagnostics.schema_version === 2">QTYPE</th><th>Entry</th><th>Peer</th><th>传输</th><th>结果</th></tr></thead>
                  <tbody>
                    <tr v-if="!selectedLog.upstream_diagnostics.attempts?.length"><td :colspan="selectedLog.upstream_diagnostics.schema_version === 2 ? 7 : 5" class="empty">无已启动上游</td></tr>
                    <tr v-for="attempt in (selectedLog.upstream_diagnostics.attempts || [])" :key="`diagnostic-${attempt.ordinal}`">
                      <td>{{ attempt.ordinal ?? '-' }}</td>
                      <td v-if="selectedLog.upstream_diagnostics.schema_version === 2">{{ attempt.branch_id ?? '-' }}</td>
                      <td v-if="selectedLog.upstream_diagnostics.schema_version === 2">{{ attempt.qtype ?? '-' }}</td>
                      <td class="mono">{{ attempt.entry || '-' }}</td>
                      <td class="mono">{{ attempt.peer || '-' }}</td>
                      <td>{{ attempt.transport || '-' }}</td>
                      <td>{{ attempt.outcome || '-' }}</td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>

          </div>
        </section>
      </div>
    </Teleport>
  </section>
</template>
