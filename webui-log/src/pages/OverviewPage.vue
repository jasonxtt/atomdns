<script setup>
import { capabilityState, operationReason, supportsOperation } from '../api/runtimeCapabilities'
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, getJSONWithHeaders, getText } from '../api/http'
import AppIcon from '../components/AppIcon.vue'
import { setError, setSuccess } from '../utils/notice'
import { formatDateTime } from '../utils/time'
import {
  cacheHitPercent,
  cacheHitTone,
  coverageText,
  cpuPercent,
  cpuTone,
  latencyTone,
  memoryTone,
  parseProcessMetrics,
  parseDistinctDomainCount
} from '../utils/overviewMetrics'
import {
  loadUpstreamGroups,
  orderUpstreamGroups,
  parseUpstreamStatsMetrics,
  protocolDisplayLabel,
  upstreamAddressDisplay,
  upstreamGroupDisplay
} from '../utils/upstreamStats'

// Client ranks return at most 500 keys; domains use exact response metadata.
const RANK_SCAN_LIMIT = 500
const TOP_ROWS = 20
// The CPU figure needs two samples; the first refresh takes a second one after this delay.
const CPU_FOLLOW_UP_MS = 3000

const loading = ref(false)

const stats = reactive({
  totalQueries: 0,
  averageDurationMs: 0,
  coverage: ''
})
const process = reactive({
  cpuPercent: null,
  residentBytes: null
})
const cacheHit = ref(null)
const clientCount = ref(null)
const domainCount = ref(null)
const learner = ref(null)

const aliases = ref({})
const topDomains = ref([])
const topClients = ref([])
const slowestQueries = ref([])
const domainSetRank = ref([])
const specialGroups = ref([])
const upstreamConfig = ref({})
// Groups not in use under the current switches (`show_when`) are left out of the statistics.
const upstreamGroupStates = ref({})
const upstreamMetricsText = ref('')
const slowDetailOpen = ref(false)
const selectedSlowQuery = ref(null)
const domainSetRankSource = ref('effective_tag')
const rankingErrors = reactive({
  stats: '',
  domain: '',
  client: '',
  slowest: '',
  rules: ''
})
const integrationErrors = reactive({
  aliases: '',
  metrics: '',
  specialGroups: '',
  upstreamConfig: '',
  learner: ''
})
const rankingDetail = reactive({
  open: false,
  type: 'domain',
  filterField: 'q',
  key: '',
  title: '',
  headline: '',
  subline: '',
  count: 0,
  percent: 0,
  loading: false,
  logs: []
})
let cpuSample = null
let cpuFollowUpTimerId = 0

const domainSetRows = computed(() => {
  const total = domainSetRank.value.reduce((sum, item) => sum + Number(item?.count || 0), 0)
  return domainSetRank.value.map((item) => {
    const count = Number(item?.count || 0)
    return {
      key: item?.key || '-',
      count,
      percent: total > 0 ? (count / total) * 100 : 0
    }
  })
})
const rankingDetailSummaryCards = computed(() => {
  if (!rankingDetail.open) {
    return []
  }

  const logs = Array.isArray(rankingDetail.logs) ? rankingDetail.logs : []
  const latestTime = findLatestLogTime(logs)
  const averageLatency = computeAverageLatency(logs)
  const maxLatency = computeMaxLatency(logs)
  const uniqueDomainCount = countUniqueValues(logs, (item) => String(item?.query_name || '').trim())
  const uniqueClientCount = countUniqueValues(logs, (item) => normalizeIP(item?.client_ip))
  const topClient = topLogValue(logs, (item) => normalizeIP(item?.client_ip))
  const topRule = topLogValue(logs, (item) => getRuleDetailValue(item))
  const topDomain = topLogValue(logs, (item) => String(item?.query_name || '').trim())
  const topUpstream = topLogValue(logs, (item) => getUpstreamDetailValue(item))

  const cards = [
    {
      key: 'count',
      title: rankingDetail.type === 'rule' ? '命中次数' : '查询次数',
      value: Number(rankingDetail.count || 0).toLocaleString()
    },
    {
      key: 'percent',
      title: '占总查询比例',
      value: formatPercent(rankingDetail.percent)
    },
    {
      key: 'latest',
      title: rankingDetail.type === 'rule' ? '最近命中时间' : '最近查询时间',
      value: latestTime ? formatTime(latestTime) : '--'
    },
    {
      key: 'avg',
      title: '平均耗时',
      value: logs.length > 0 ? formatDuration(averageLatency) : '--'
    },
    {
      key: 'max',
      title: '最大耗时',
      value: logs.length > 0 ? formatDuration(maxLatency) : '--'
    }
  ]

  if (rankingDetail.type === 'domain') {
    cards.push(
      {
        key: 'client',
        title: '主要客户端',
        value: topClient ? getClientDisplay(topClient.key) : '--'
      },
      {
        key: 'rule',
        title: '主要分流规则',
        value: topRule ? getDetailRuleLabel(topRule.key) : '--'
      }
    )
  } else if (rankingDetail.type === 'client') {
    cards.push(
      {
        key: 'domains',
        title: '唯一域名数量',
        value: uniqueDomainCount.toLocaleString()
      },
      {
        key: 'rule',
        title: '主要分流规则',
        value: topRule ? getDetailRuleLabel(topRule.key) : '--'
      },
      {
        key: 'upstream',
        title: '主要上游',
        value: topUpstream ? topUpstream.key : '--'
      }
    )
  } else if (rankingDetail.type === 'rule') {
    cards.push(
      {
        key: 'domains',
        title: '唯一域名数量',
        value: uniqueDomainCount.toLocaleString()
      },
      {
        key: 'clients',
        title: '唯一客户端数量',
        value: uniqueClientCount.toLocaleString()
      },
      {
        key: 'domain',
        title: '主要域名',
        value: topDomain ? topDomain.key : '--'
      },
      {
        key: 'client',
        title: '主要客户端',
        value: topClient ? getClientDisplay(topClient.key) : '--'
      }
    )
  }

  return cards
})
// Rows of a ranking card: the bar is the row's value against the largest one in that list.
function withBars(items, valueOf) {
  const max = items.reduce((peak, item) => Math.max(peak, Number(valueOf(item)) || 0), 0)
  return items.map((item) => ({ item, width: max > 0 ? Math.max(2, ((Number(valueOf(item)) || 0) / max) * 100) : 0 }))
}
const topDomainBars = computed(() => withBars(topDomains.value, (item) => item.count))
const topClientBars = computed(() => withBars(topClients.value, (item) => item.count))
const slowestBars = computed(() => withBars(slowestQueries.value, (item) => item.duration_ms))

// 分流统计 donut: the five largest rules, the rest as one slice; r=50 on a 120 box.
const DONUT_SLICES = 5
const DONUT_CIRCUMFERENCE = 2 * Math.PI * 50
const domainSetDonut = computed(() => {
  const rows = domainSetRows.value
  const slices = rows.slice(0, DONUT_SLICES).map((item) => ({ key: item.key, label: getRuleLabel(item.key), count: item.count, percent: item.percent, item }))
  const rest = rows.slice(DONUT_SLICES)
  if (rest.length > 0) {
    const count = rest.reduce((sum, item) => sum + item.count, 0)
    slices.push({ key: '__other', label: '其他', count, percent: rest.reduce((sum, item) => sum + item.percent, 0), item: null })
  }
  let offset = 0
  const segments = slices.map((slice, index) => {
    const length = (slice.percent / 100) * DONUT_CIRCUMFERENCE
    const segment = { ...slice, color: `var(--rp-c${index + 1})`, dash: `${length} ${DONUT_CIRCUMFERENCE - length}`, offset: -offset }
    offset += length
    return segment
  })
  return { total: rows.reduce((sum, item) => sum + item.count, 0), segments }
})
const slowDetailActionFields = computed(() => {
  const log = selectedSlowQuery.value
  if (!log) {
    return {}
  }
  return {
    client_ip: {
      value: getClientDisplay(log.client_ip),
      copyValue: normalizeIP(log.client_ip),
      filterValue: normalizeIP(log.client_ip),
      exact: true,
      mono: false
    },
    query_name: {
      value: log.query_name || '-',
      copyValue: String(log.query_name || '').trim(),
      filterValue: String(log.query_name || '').trim(),
      exact: false,
      mono: false
    },
    domain_set: {
      value: getDetailRuleLabel(log.domain_set || log.effective_tag || '-'),
      copyValue: String(log.domain_set || '').trim(),
      filterValue: String(log.domain_set || '').trim(),
      exact: true,
      mono: false
    },
    trace_id: {
      value: log.trace_id || '-',
      copyValue: String(log.trace_id || '').trim(),
      filterValue: String(log.trace_id || '').trim(),
      exact: true,
      mono: true
    }
  }
})

function distinctCountText(count) {
  if (count === null) return '--'
  return count >= RANK_SCAN_LIMIT ? `${RANK_SCAN_LIMIT}+` : count.toLocaleString('zh-CN')
}

const STAT_ICONS = { queries: 'activity', latency: 'timer', cache: 'database', clients: 'users', domains: 'globe', cpu: 'cpu', memory: 'memory' }
const statCards = computed(() => {
  const coverage = stats.coverage || '审计保留记录'
  const memoryMiB = process.residentBytes === null ? null : process.residentBytes / 1024 / 1024
  const processReason = supportsOperation('metrics.process') ? '' : operationReason('metrics.process')
  return [
    { key: 'queries', title: '查询数量', value: rankingErrors.stats ? '--' : stats.totalQueries.toLocaleString('zh-CN'), note: coverage, tone: '' },
    { key: 'latency', title: '平均响应', value: rankingErrors.stats ? '--' : `${stats.averageDurationMs.toFixed(1)} ms`, note: coverage, tone: rankingErrors.stats ? '' : latencyTone(stats.averageDurationMs) },
    { key: 'cache', title: '缓存命中', value: cacheHit.value === null ? '--' : `${cacheHit.value.toFixed(1)}%`, note: '全部命名缓存的累计计数', tone: cacheHitTone(cacheHit.value) },
    { key: 'clients', title: '客户端数量', value: distinctCountText(clientCount.value), note: coverage, tone: '' },
    { key: 'domains', title: '域名统计', value: domainCount.value === null ? '--' : domainCount.value.toLocaleString('zh-CN'), note: `不同域名 · ${coverage}`, tone: '' },
    { key: 'cpu', title: 'CPU 占用', value: process.cpuPercent === null ? '--' : `${process.cpuPercent.toFixed(1)}%`, note: processReason || '单核百分比 · 两次刷新之间', tone: cpuTone(process.cpuPercent) },
    { key: 'memory', title: '内存占用', value: memoryMiB === null ? '--' : `${memoryMiB.toFixed(1)} MiB`, note: processReason || '常驻内存 (RSS)', tone: memoryTone(process.residentBytes) }
  ]
})

const learnerCategories = computed(() => (Array.isArray(learner.value?.categories) ? learner.value.categories : []))
const learnerTotal = computed(() => learnerCategories.value.reduce((sum, item) => sum + Number(item?.entries || 0), 0))

function toneClass(tone) {
  return tone ? `rt-text-${tone}` : ''
}

const upstreamStatsMap = computed(() => parseUpstreamStatsMetrics(upstreamMetricsText.value))
const upstreamStatSections = computed(() => {
  const configuredGroups = Object.keys(upstreamConfig.value || {}).filter((group) => upstreamGroupStates.value[group]?.active !== false)
  const orderedGroups = orderUpstreamGroups(configuredGroups, specialGroups.value)
  const sections = []
  orderedGroups.forEach((group) => {
    const rows = Array.isArray(upstreamConfig.value?.[group]) ? upstreamConfig.value[group] : []
    const enabledRows = rows
      .filter((item) => Boolean(item?.enabled))
      .map((item, index) => {
        const key = `${group}|${item?.tag || ''}`
        const stats = upstreamStatsMap.value[key] || {}
        const queryNumber = Number(stats.queryTotal || 0)
        const winnerNumber = Number(stats.winnerTotal || 0)
        const errorNumber = Number(stats.errorTotal || 0)
        const latencySum = Number(stats.latencySum || 0)
        const latencyCount = Number(stats.latencyCount || 0)
        const avgLatencyNumber = latencyCount > 0 ? latencySum / latencyCount : 0
        const winnerRateNumber = queryNumber > 0 ? (winnerNumber / queryNumber) * 100 : 0
        const errorRateNumber = queryNumber > 0 ? (errorNumber / queryNumber) * 100 : 0

        return {
          key: `${group}-${item?.tag || index}`,
          name: String(item?.tag || '-'),
          typeLabel: protocolDisplayLabel(item?.protocol),
          address: upstreamAddressDisplay(item || {}),
          avgLatencyText: latencyCount > 0 ? avgLatencyNumber.toFixed(2) : '-',
          avgLatencyNumber,
          latencyTone: latencyCount > 0 ? latencyTone(avgLatencyNumber) : '',
          queryText: queryNumber.toLocaleString(),
          queryNumber,
          winnerRateText: `${winnerRateNumber.toFixed(2)}%`,
          winnerRateNumber,
          errorNumber,
          errorRateText: `${errorRateNumber.toFixed(2)}%`,
          errorRateNumber
        }
      })

    if (enabledRows.length === 0) {
      return
    }

    const groupMeta = upstreamGroupDisplay(group, specialGroups.value)
    sections.push({
      key: group,
      title: groupMeta.title,
      rows: enabledRows
    })
  })

  return sections
})

function normalizeIP(ip) {
  return String(ip || '').replace(/^::ffff:/, '').trim()
}

function normalizeAliasMap(input) {
  const output = {}
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    return output
  }
  Object.entries(input).forEach(([key, value]) => {
    const ip = normalizeIP(key)
    const alias = String(value || '').trim()
    if (ip && alias) {
      output[ip] = alias
    }
  })
  return output
}

function getClientDisplay(ip) {
  const normalized = normalizeIP(ip)
  if (!normalized) {
    return '-'
  }
  const alias = aliases.value[normalized]
  return alias || normalized
}

function hasAlias(ip) {
  const normalized = normalizeIP(ip)
  return Boolean(normalized && aliases.value[normalized] && aliases.value[normalized] !== normalized)
}

function formatDuration(value) {
  const num = Number(value || 0)
  return `${num.toFixed(2)} ms`
}

function formatCompactDuration(value) {
  const num = Number(value || 0)
  if (num >= 100) {
    return String(Math.round(num))
  }
  if (num >= 10) {
    return num.toFixed(1)
  }
  return num.toFixed(2)
}


function formatTime(value) {
  return formatDateTime(value)
}

function getRuleLabel(key) {
  if (!key) {
    return '-'
  }
  const match = String(key).match(/^特殊上游(\d+)$/)
  if (!match) {
    return key
  }
  const slot = Number(match[1])
  const group = specialGroups.value.find((item) => Number(item.slot) === slot)
  return group?.name || key
}

function formatPercent(value) {
  return `${Number(value || 0).toFixed(1)}%`
}

function formatResponseFlags(flags) {
  if (!flags || typeof flags !== 'object') {
    return '-'
  }
  const enabled = [
    ['RA', flags.RA ?? flags.ra],
    ['AA', flags.AA ?? flags.aa],
    ['TC', flags.TC ?? flags.tc],
    ['AD', flags.AD ?? flags.ad],
    ['CD', flags.CD ?? flags.cd]
  ]
    .filter(([, value]) => Boolean(value))
    .map(([key]) => key)
  return enabled.length > 0 ? enabled.join(', ') : '-'
}

function getDetailRuleLabel(value) {
  if (!value) {
    return '-'
  }
  return getRuleLabel(value)
}

function getRuleDetailValue(log) {
  return String(log?.effective_tag || log?.domain_set || '').trim()
}

function getUpstreamDetailValue(log) {
  return String(log?.selected_upstream || log?.final_upstream || log?.upstream_group || '-').trim()
}

function findLatestLogTime(logs) {
  return logs.reduce((latest, item) => {
    const timestamp = new Date(item?.query_time || 0).getTime()
    if (!Number.isFinite(timestamp) || timestamp <= 0) {
      return latest
    }
    return Math.max(latest, timestamp)
  }, 0)
}

function computeAverageLatency(logs) {
  if (!Array.isArray(logs) || logs.length === 0) {
    return 0
  }
  const total = logs.reduce((sum, item) => sum + Number(item?.duration_ms || 0), 0)
  return total / logs.length
}

function computeMaxLatency(logs) {
  if (!Array.isArray(logs) || logs.length === 0) {
    return 0
  }
  return logs.reduce((max, item) => Math.max(max, Number(item?.duration_ms || 0)), 0)
}

function countUniqueValues(logs, selector) {
  const values = new Set()
  logs.forEach((item) => {
    const key = String(selector(item) || '').trim()
    if (key) {
      values.add(key)
    }
  })
  return values.size
}

function topLogValue(logs, selector) {
  const counts = new Map()
  logs.forEach((item) => {
    const key = String(selector(item) || '').trim()
    if (!key) {
      return
    }
    counts.set(key, (counts.get(key) || 0) + 1)
  })
  let bestKey = ''
  let bestCount = 0
  counts.forEach((count, key) => {
    if (count > bestCount) {
      bestKey = key
      bestCount = count
    }
  })
  return bestKey ? { key: bestKey, count: bestCount } : null
}

function calculateRankingPercent(count) {
  const total = Number(stats.totalQueries || 0)
  if (total <= 0) {
    return 0
  }
  return (Number(count || 0) / total) * 100
}

function openSlowDetail(item) {
  selectedSlowQuery.value = item || null
  slowDetailOpen.value = Boolean(item)
}

function closeSlowDetail() {
  slowDetailOpen.value = false
  selectedSlowQuery.value = null
}

async function copyText(text) {
  const value = String(text || '').trim()
  if (!value) {
    setError('没有可复制的内容')
    return
  }
  try {
    if (navigator.clipboard?.writeText && window.isSecureContext) {
      await navigator.clipboard.writeText(value)
    } else {
      const textArea = document.createElement('textarea')
      textArea.value = value
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

function getSlowDetailActionField(key) {
  return slowDetailActionFields.value?.[key] || null
}

function openLogFilterFromOverview(value, exact = false) {
  const text = String(value || '').trim()
  if (!text) {
    setError('没有可筛选的内容')
    return
  }
  closeSlowDetail()
  closeRankingDetail()
  window.dispatchEvent(new CustomEvent('mosdns-open-log-filter', {
    detail: {
      value: text,
      exact: Boolean(exact)
    }
  }))
}

async function loadRankingDetailLogs() {
  const detailKey = rankingDetail.key
  const filterField = rankingDetail.filterField
  rankingDetail.loading = true
  try {
    const params = new URLSearchParams({
      page: '1',
      limit: '50'
    })
    let endpoint = '/api/v2/audit/logs'
    if (rankingDetail.type === 'domain') {
      endpoint = '/api/v2/audit/logs/domain'
      params.set('domain', detailKey)
    } else if (filterField === 'q') {
      params.set('q', detailKey)
      params.set('exact', 'true')
    } else {
      params.set(filterField, detailKey)
    }
    let data
    try {
      data = await getJSON(`${endpoint}?${params.toString()}`)
    } catch (error) {
      if (rankingDetail.type === 'domain' && error?.status === 404) {
        params.delete('domain')
        params.set('q', detailKey)
        params.set('exact', 'true')
        data = await getJSON(`/api/v2/audit/logs?${params.toString()}`)
      } else {
        throw error
      }
    }
    if (!rankingDetail.open || rankingDetail.key !== detailKey || rankingDetail.filterField !== filterField) {
      return
    }
    rankingDetail.logs = Array.isArray(data?.logs) ? data.logs : []
  } catch (error) {
    rankingDetail.logs = []
    setError(`加载详情失败: ${error.message}`)
  } finally {
    if (rankingDetail.key === detailKey && rankingDetail.filterField === filterField) {
      rankingDetail.loading = false
    }
  }
}

function openRankingDetail(type, item) {
  const rawKey = String(item?.key || '').trim()
  const count = Number(item?.count || 0)
  if (!rawKey) {
    return
  }
  rankingDetail.type = type
  rankingDetail.key = rawKey
  rankingDetail.count = count
  rankingDetail.percent = calculateRankingPercent(count)
  rankingDetail.logs = []
  rankingDetail.loading = false

  if (type === 'domain') {
    rankingDetail.title = '域名详情'
    rankingDetail.headline = rawKey
    rankingDetail.subline = ''
    rankingDetail.filterField = 'q'
  } else if (type === 'client') {
    rankingDetail.title = '客户端详情'
    rankingDetail.headline = getClientDisplay(rawKey)
    rankingDetail.subline = hasAlias(rawKey) ? normalizeIP(rawKey) : ''
    rankingDetail.filterField = 'client_ip'
    rankingDetail.key = normalizeIP(rawKey)
  } else {
    const filterField = domainSetRankSource.value === 'domain_set' ? 'domain_set' : 'effective_tag'
    rankingDetail.title = '分流规则详情'
    rankingDetail.headline = getRuleLabel(rawKey)
    rankingDetail.subline = `${filterField}: ${rawKey}`
    rankingDetail.filterField = filterField
  }

  rankingDetail.open = true
  loadRankingDetailLogs()
}

function openTopDomainDetail(item) {
  openRankingDetail('domain', item)
}

function openTopClientDetail(item) {
  openRankingDetail('client', item)
}

function openDomainSetDetail(item) {
  openRankingDetail('rule', item)
}

function closeRankingDetail() {
  rankingDetail.open = false
  rankingDetail.key = ''
  rankingDetail.headline = ''
  rankingDetail.subline = ''
  rankingDetail.logs = []
  rankingDetail.loading = false
}

// `at`: when the metrics text arrived, so slow requests of the same refresh do not stretch the interval.
function applyProcessMetrics(metricsText, at) {
  const figures = parseProcessMetrics(metricsText)
  process.residentBytes = figures.residentBytes
  if (figures.cpuSeconds !== null) {
    const sample = { cpuSeconds: figures.cpuSeconds, at }
    const percent = cpuPercent(cpuSample, sample)
    if (percent !== null || cpuSample === null) process.cpuPercent = percent
    cpuSample = sample
  }
}

async function sampleCpuOnly() {
  cpuFollowUpTimerId = 0
  if (!supportsOperation('metrics.cache') || !supportsOperation('metrics.process')) return
  try {
    applyProcessMetrics(await getText('/metrics'), Date.now())
  } catch {
    // the next refresh tries again
  }
}

async function reloadOverview(showMessage = false) {
  if (capabilityState.status !== 'ready') return
  loading.value = true
  try {
    const auditPanel = (promise) => promise
      .then((data) => ({ data, error: '', status: 200 }))
      .catch((error) => ({
        data: null,
        error: error?.message || '请求失败',
        status: Number(error?.status || 0)
      }))
    const optionalPanel = (operation, request) => supportsOperation(operation) ? auditPanel(request()) : Promise.resolve({ data: null, error: '', status: 0 })
    const rankPanels = async () => {
      if (!supportsOperation('query.rank')) return Object.fromEntries(['domain', 'client', 'slowest', 'rules'].map(key => [key, { data: [], error: '', status: 0 }]))
      // Audit ranks use a two-slot, no-queue backend gate: one refresh asks them one
      // after another so it cannot manufacture its own 503s.
      return {
        domain: await auditPanel(getJSONWithHeaders(`/api/v2/audit/rank/domain?limit=${TOP_ROWS}`)),
        client: await auditPanel(getJSON(`/api/v2/audit/rank/client?limit=${RANK_SCAN_LIMIT}`)),
        slowest: await auditPanel(getJSON(`/api/v2/audit/rank/slowest?limit=${TOP_ROWS}`)),
        rules: await auditPanel(getJSON(`/api/v2/audit/rank/effective?limit=${TOP_ROWS}`))
      }
    }

    const [
      statsRes,
      windowsRes,
      rankRes,
      specialGroupsRes,
      aliasesRes,
      upstreamConfigRes,
      metricsRes,
      learnerRes
    ] = await Promise.all([
      optionalPanel('audit.read', () => getJSON('/api/v2/audit/stats')),
      optionalPanel('audit.read', () => getJSON('/api/v2/audit/stats/windows')),
      rankPanels(),
      optionalPanel('groups.read', () => getJSON('/api/v1/special-groups')),
      optionalPanel('client.aliases', () => getJSON('/plugins/clientname')),
      optionalPanel('upstreams.read', () => getJSON('/api/v1/upstream/config')),
      optionalPanel('metrics.cache', () => getText('/metrics').then((text) => ({ text, at: Date.now() }))),
      optionalPanel('lists.remembered', () => getJSON('/plugins/route_learner/status'))
    ])

    rankingErrors.stats = statsRes.error
    rankingErrors.domain = rankRes.domain.error
    rankingErrors.client = rankRes.client.error
    rankingErrors.slowest = rankRes.slowest.error
    rankingErrors.rules = rankRes.rules.error
    integrationErrors.specialGroups = specialGroupsRes.error
    integrationErrors.aliases = aliasesRes.error
    integrationErrors.upstreamConfig = upstreamConfigRes.error
    integrationErrors.metrics = metricsRes.error
    integrationErrors.learner = learnerRes.error
    if (!statsRes.error) {
      stats.totalQueries = Number(statsRes?.data?.total_queries || 0)
      stats.averageDurationMs = Number(statsRes?.data?.average_duration_ms || 0)
    }
    // The windows answer carries the oldest retained record: the coverage of every figure.
    const windows = windowsRes.error ? null : windowsRes.data
    stats.coverage = windows ? coverageText(windows.items?.[0]?.coverage_start, windows.generated_at, stats.totalQueries) : ''

    const domains = Array.isArray(rankRes.domain.data?.data) ? rankRes.domain.data.data : []
    const clients = Array.isArray(rankRes.client.data) ? rankRes.client.data : []
    topDomains.value = domains.slice(0, TOP_ROWS)
    topClients.value = clients.slice(0, TOP_ROWS)
    domainCount.value = rankRes.domain.error || !supportsOperation('query.rank') ? null : parseDistinctDomainCount(rankRes.domain.data?.headers?.get('X-Audit-Distinct-Domains'))
    clientCount.value = rankRes.client.error || !supportsOperation('query.rank') ? null : clients.length
    slowestQueries.value = Array.isArray(rankRes.slowest.data) ? rankRes.slowest.data : []
    domainSetRank.value = Array.isArray(rankRes.rules.data) ? rankRes.rules.data : []
    domainSetRankSource.value = 'effective_tag'
    specialGroups.value = Array.isArray(specialGroupsRes.data) ? specialGroupsRes.data : []
    aliases.value = normalizeAliasMap(aliasesRes.data)
    upstreamConfig.value = upstreamConfigRes.data && typeof upstreamConfigRes.data === 'object' ? upstreamConfigRes.data : {}
    upstreamGroupStates.value = upstreamConfigRes.error ? {} : await loadUpstreamGroups(getJSON, capabilityState.value)
    upstreamMetricsText.value = metricsRes.error ? '' : String(metricsRes.data?.text || '')
    learner.value = learnerRes.error ? null : learnerRes.data
    if (!metricsRes.error && metricsRes.data !== null) {
      cacheHit.value = cacheHitPercent(metricsRes.data.text)
      if (supportsOperation('metrics.process')) {
        const first = cpuSample === null
        applyProcessMetrics(metricsRes.data.text, metricsRes.data.at)
        if (first && !cpuFollowUpTimerId) cpuFollowUpTimerId = window.setTimeout(sampleCpuOnly, CPU_FOLLOW_UP_MS)
      }
    }

    if (showMessage) {
      setSuccess('概览数据已刷新')
    }
  } catch (error) {
    setError(`加载概览失败: ${error.message}`)
  } finally {
    loading.value = false
  }
}

function handleGlobalRefresh() {
  reloadOverview(false)
}

onMounted(() => {
  reloadOverview(false)
  window.addEventListener('mosdns-log-refresh', handleGlobalRefresh)
})

onBeforeUnmount(() => {
  if (cpuFollowUpTimerId) window.clearTimeout(cpuFollowUpTimerId)
  window.removeEventListener('mosdns-log-refresh', handleGlobalRefresh)
})
</script>

<template>
  <section class="overview-page">
    <div class="rt-stat-grid" data-overview-stats>
      <article v-for="card in statCards" :key="card.key" class="rt-stat" :data-stat="card.key">
        <span class="rt-stat-label"><AppIcon :name="STAT_ICONS[card.key]" :size="18" />{{ card.title }}</span>
        <strong class="rt-stat-value" :class="toneClass(card.tone)">{{ card.value }}</strong>
        <small class="rt-stat-note">{{ card.note }}</small>
      </article>
    </div>

    <div v-if="rankingErrors.stats" class="overview-unavailable" role="status">
      查询统计不可用：{{ rankingErrors.stats }}
    </div>

    <div class="rt-overview-row">
      <section class="panel sub-panel upstream-stats-panel">
        <header class="upstream-stats-head">
          <div class="upstream-stats-title-wrap">
            <h3>上游 DNS 统计</h3>
          </div>
        </header>
        <div v-if="integrationErrors.metrics || integrationErrors.upstreamConfig" class="overview-unavailable" role="status">
          <span v-if="integrationErrors.metrics">指标不可用：{{ integrationErrors.metrics }}</span>
          <span v-if="integrationErrors.upstreamConfig">上游配置不可用：{{ integrationErrors.upstreamConfig }}</span>
        </div>
        <div class="flat-table-wrap">
          <table class="flat-table upstream-stats-table">
            <thead>
              <tr>
                <th>名称</th>
                <th class="text-right" title="平均响应时间（毫秒）">平均 ms</th>
                <th class="text-right">请求数</th>
                <th class="text-right">采纳率</th>
                <th class="text-right">出错率</th>
              </tr>
            </thead>
            <tbody v-if="loading">
              <tr>
                <td colspan="5" class="empty">加载中...</td>
              </tr>
            </tbody>
            <tbody v-else-if="upstreamStatSections.length === 0">
              <tr>
                <td colspan="5" class="empty">暂无已启用上游统计</td>
              </tr>
            </tbody>
            <tbody v-for="section in upstreamStatSections" v-else :key="section.key">
              <tr class="upstream-stats-group-row">
                <td colspan="5">
                  <span class="upstream-stats-group-name">{{ section.title }}</span>
                  <span class="upstream-stats-group-count">{{ section.rows.length }}</span>
                </td>
              </tr>
              <tr v-for="row in section.rows" :key="row.key">
                <td class="upstream-stats-name-cell" :title="`${row.typeLabel} · ${row.address}`">{{ row.name }}</td>
                <td class="text-right" :class="toneClass(row.latencyTone)" :title="row.avgLatencyText === '-' ? '-' : `${row.avgLatencyText} ms`">
                  {{ row.avgLatencyText }}
                </td>
                <td class="text-right">{{ row.queryText }}</td>
                <td class="text-right">{{ row.winnerRateText }}</td>
                <td class="text-right">{{ row.errorRateText }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
      <section class="panel sub-panel rank-card" data-overview-rules>
        <h3>分流统计</h3>
        <div v-if="integrationErrors.specialGroups" class="overview-unavailable" role="status">
          特殊分组名称不可用：{{ integrationErrors.specialGroups }}；以下显示原始标签
        </div>
        <p v-if="rankingErrors.rules" class="muted">{{ rankingErrors.rules }}</p>
        <p v-else-if="domainSetDonut.segments.length === 0" class="muted">暂无数据</p>
        <div v-else class="donut-card">
          <div class="donut-wrap">
            <svg class="donut-svg" viewBox="0 0 120 120" aria-hidden="true">
              <circle class="donut-track" cx="60" cy="60" r="50" />
              <circle
                v-for="segment in domainSetDonut.segments"
                :key="segment.key"
                class="donut-segment"
                cx="60"
                cy="60"
                r="50"
                :stroke="segment.color"
                :stroke-dasharray="segment.dash"
                :stroke-dashoffset="segment.offset"
              />
            </svg>
            <div class="donut-center">
              <strong>{{ domainSetDonut.total.toLocaleString() }}</strong>
              <span>次</span>
            </div>
          </div>
          <ul class="donut-legend">
            <li v-for="segment in domainSetDonut.segments" :key="segment.key">
              <component :is="segment.item ? 'button' : 'div'" class="legend-row" :type="segment.item ? 'button' : undefined" @click="segment.item && openDomainSetDetail(segment.item)">
                <i class="legend-dot" :style="{ background: segment.color }"></i>
                <span class="legend-name" :title="`${segment.label} · ${segment.count.toLocaleString()} 次`">{{ segment.label }}</span>
                <span class="legend-pct">{{ formatPercent(segment.percent) }}</span>
              </component>
            </li>
          </ul>
        </div>
      </section>
    </div>

    <div class="overview-ranks">
      <section class="panel sub-panel rank-card">
        <h3>Top 域名</h3>
        <p v-if="rankingErrors.domain" class="muted">{{ rankingErrors.domain }}</p>
        <p v-else-if="topDomainBars.length === 0" class="muted">暂无数据</p>
        <ul v-else class="rank-list">
          <li v-for="row in topDomainBars" :key="`domain-${row.item.key}`">
            <button type="button" class="rank-row" @click="openTopDomainDetail(row.item)">
              <span class="rank-label">{{ row.item.key }}</span>
              <span class="rank-value">{{ Number(row.item.count || 0).toLocaleString() }}</span>
              <AppIcon class="rank-go" name="chevron-right" :size="16" />
              <span class="rank-bar"><i :style="{ width: `${row.width}%` }"></i></span>
            </button>
          </li>
        </ul>
      </section>

      <section class="panel sub-panel rank-card">
        <h3>Top 客户端</h3>
        <div v-if="integrationErrors.aliases" class="overview-unavailable" role="status">
          客户端别名不可用：{{ integrationErrors.aliases }}；以下显示原始地址
        </div>
        <p v-if="rankingErrors.client" class="muted">{{ rankingErrors.client }}</p>
        <p v-else-if="topClientBars.length === 0" class="muted">暂无数据</p>
        <ul v-else class="rank-list">
          <li v-for="row in topClientBars" :key="`client-${row.item.key}`">
            <button type="button" class="rank-row" @click="openTopClientDetail(row.item)">
              <span class="rank-label">
                {{ getClientDisplay(row.item.key) }}
                <small v-if="hasAlias(row.item.key)" class="muted mono">{{ normalizeIP(row.item.key) }}</small>
              </span>
              <span class="rank-value">{{ Number(row.item.count || 0).toLocaleString() }}</span>
              <AppIcon class="rank-go" name="chevron-right" :size="16" />
              <span class="rank-bar rank-bar-iris"><i :style="{ width: `${row.width}%` }"></i></span>
            </button>
          </li>
        </ul>
      </section>

      <section class="panel sub-panel rank-card">
        <h3>最慢查询</h3>
        <p v-if="rankingErrors.slowest" class="muted">{{ rankingErrors.slowest }}</p>
        <p v-else-if="slowestBars.length === 0" class="muted">暂无数据</p>
        <ul v-else class="rank-list">
          <li v-for="(row, index) in slowestBars" :key="`slow-${index}-${row.item.trace_id || row.item.query_time}`">
            <button type="button" class="rank-row" @click="openSlowDetail(row.item)">
              <span class="rank-label">{{ row.item.query_name }}</span>
              <span class="rank-value" :title="formatDuration(row.item.duration_ms)">{{ formatCompactDuration(row.item.duration_ms) }} ms</span>
              <AppIcon class="rank-go" name="chevron-right" :size="16" />
              <span class="rank-bar rank-bar-gold"><i :style="{ width: `${row.width}%` }"></i></span>
            </button>
          </li>
        </ul>
      </section>

      <section class="panel sub-panel rank-card" data-overview-learner>
        <h3>学习状态</h3>
        <p v-if="!supportsOperation('lists.remembered')" class="muted">当前配置没有域名学习。</p>
        <p v-else-if="integrationErrors.learner" class="muted">学习状态不可用：{{ integrationErrors.learner }}</p>
        <template v-else-if="learner">
          <p class="muted learner-summary">共 {{ learnerTotal.toLocaleString('zh-CN') }} 条 · 启动以来新学 {{ Number(learner.counters?.learned || 0).toLocaleString('zh-CN') }} · 过期 {{ Number(learner.counters?.expired || 0).toLocaleString('zh-CN') }}</p>
          <div class="flat-table-wrap">
            <table class="flat-table learner-table">
              <thead>
                <tr>
                  <th>类别</th>
                  <th class="text-right">条数</th>
                  <th class="text-right">上限</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="item in learnerCategories" :key="item.tag">
                  <td>{{ item.label || item.tag }}<span v-if="!item.enabled" class="muted rt-inline-note">（已关闭）</span></td>
                  <td class="text-right">{{ Number(item.entries || 0).toLocaleString('zh-CN') }}</td>
                  <td class="text-right">{{ Number(item.max_entries || 0).toLocaleString('zh-CN') }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </template>
        <p v-else class="muted">读取中…</p>
      </section>
    </div>

    <Teleport to="body">
      <div v-if="slowDetailOpen && selectedSlowQuery" class="modal-mask modal-mask-top" @click.self="closeSlowDetail">
        <section class="panel data-view-modal overview-slow-detail-modal">
          <header class="panel-header">
            <div>
              <h3>最慢查询详情</h3>
            </div>
            <div class="actions">
              <button class="btn secondary" type="button" @click="closeSlowDetail">关闭</button>
            </div>
          </header>
          <div class="detail-grid">
            <div><strong>时间:</strong> {{ formatTime(selectedSlowQuery.query_time) }}</div>
            <div>
              <strong>客户端:</strong>
              <span :class="{ mono: getSlowDetailActionField('client_ip')?.mono }">{{ getSlowDetailActionField('client_ip')?.value || '-' }}</span>
              <span class="detail-inline-actions">
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('client_ip')?.copyValue" @click="copyText(getSlowDetailActionField('client_ip')?.copyValue)">复制</button>
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('client_ip')?.filterValue" @click="openLogFilterFromOverview(getSlowDetailActionField('client_ip')?.filterValue, getSlowDetailActionField('client_ip')?.exact)">筛选</button>
              </span>
            </div>
            <div>
              <strong>域名:</strong>
              <span :class="{ mono: getSlowDetailActionField('query_name')?.mono }">{{ getSlowDetailActionField('query_name')?.value || '-' }}</span>
              <span class="detail-inline-actions">
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('query_name')?.copyValue" @click="copyText(getSlowDetailActionField('query_name')?.copyValue)">复制</button>
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('query_name')?.filterValue" @click="openLogFilterFromOverview(getSlowDetailActionField('query_name')?.filterValue, getSlowDetailActionField('query_name')?.exact)">筛选</button>
              </span>
            </div>
            <div><strong>类型:</strong> {{ selectedSlowQuery.query_type || '-' }}</div>
            <div><strong>类别:</strong> {{ selectedSlowQuery.query_class || '-' }}</div>
            <div>
              <strong>Trace ID:</strong>
              <span class="mono">{{ getSlowDetailActionField('trace_id')?.value || '-' }}</span>
              <span class="detail-inline-actions">
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('trace_id')?.copyValue" @click="copyText(getSlowDetailActionField('trace_id')?.copyValue)">复制</button>
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('trace_id')?.filterValue" @click="openLogFilterFromOverview(getSlowDetailActionField('trace_id')?.filterValue, getSlowDetailActionField('trace_id')?.exact)">筛选</button>
              </span>
            </div>
            <div><strong>生效标签:</strong> {{ getDetailRuleLabel(selectedSlowQuery.effective_tag || selectedSlowQuery.domain_set) }}</div>
            <div>
              <strong>分流规则:</strong>
              <span>{{ getSlowDetailActionField('domain_set')?.value || '-' }}</span>
              <span class="detail-inline-actions">
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('domain_set')?.copyValue" @click="copyText(getSlowDetailActionField('domain_set')?.copyValue)">复制</button>
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('domain_set')?.filterValue" @click="openLogFilterFromOverview(getSlowDetailActionField('domain_set')?.filterValue, getSlowDetailActionField('domain_set')?.exact)">筛选</button>
              </span>
            </div>
            <div><strong>上游组:</strong> {{ selectedSlowQuery.final_upstream || selectedSlowQuery.upstream_group || '-' }}</div>
            <div><strong>最终上游:</strong> {{ selectedSlowQuery.selected_upstream || '-' }}</div>
            <div v-if="selectedSlowQuery.upstream_diagnostics && ![1, 2].includes(selectedSlowQuery.upstream_diagnostics.schema_version)" class="detail-wide muted">
              原生上游诊断版本暂不支持（{{ selectedSlowQuery.upstream_diagnostics.schema_version ?? '-' }}）；普通响应与路由信息仍可用。
            </div>
            <div v-if="[1, 2].includes(selectedSlowQuery.upstream_diagnostics?.schema_version)" class="detail-wide">
              <strong>原生上游诊断:</strong>
              <div v-if="selectedSlowQuery.upstream_diagnostics.schema_version === 2 && selectedSlowQuery.upstream_diagnostics.branches?.length" class="table-wrap upstream-diagnostics-table">
                <table>
                  <thead><tr><th>ID</th><th>父级</th><th>角色</th><th>策略</th><th>QTYPE</th><th>决定</th></tr></thead>
                  <tbody>
                    <tr v-for="branch in selectedSlowQuery.upstream_diagnostics.branches" :key="`slow-branch-${branch.id}`">
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
              <span v-if="selectedSlowQuery.upstream_diagnostics.selected">
                选中 {{ selectedSlowQuery.upstream_diagnostics.selected.entry || '-' }} ·
                {{ selectedSlowQuery.upstream_diagnostics.selected.peer || '-' }} ·
                {{ selectedSlowQuery.upstream_diagnostics.selected.transport || '-' }}
              </span>
              <span v-else>未选中最终网络上游</span>
              <div class="table-wrap upstream-diagnostics-table">
                <table>
                  <thead><tr><th>序号</th><th v-if="selectedSlowQuery.upstream_diagnostics.schema_version === 2">分支</th><th v-if="selectedSlowQuery.upstream_diagnostics.schema_version === 2">QTYPE</th><th>Entry</th><th>Peer</th><th>传输</th><th>结果</th></tr></thead>
                  <tbody>
                    <tr v-if="!selectedSlowQuery.upstream_diagnostics.attempts?.length"><td :colspan="selectedSlowQuery.upstream_diagnostics.schema_version === 2 ? 7 : 5" class="empty">无已启动上游</td></tr>
                    <tr v-for="attempt in (selectedSlowQuery.upstream_diagnostics.attempts || [])" :key="`slow-diagnostic-${attempt.ordinal}`">
                      <td>{{ attempt.ordinal ?? '-' }}</td>
                      <td v-if="selectedSlowQuery.upstream_diagnostics.schema_version === 2">{{ attempt.branch_id ?? '-' }}</td>
                      <td v-if="selectedSlowQuery.upstream_diagnostics.schema_version === 2">{{ attempt.qtype ?? '-' }}</td>
                      <td class="mono">{{ attempt.entry || '-' }}</td>
                      <td class="mono">{{ attempt.peer || '-' }}</td>
                      <td>{{ attempt.transport || '-' }}</td>
                      <td>{{ attempt.outcome || '-' }}</td>
                    </tr>
                  </tbody>
                </table>
              </div>
            </div>
            <div><strong>响应码:</strong> {{ selectedSlowQuery.response_code || '-' }}</div>
            <div><strong>响应标志:</strong> {{ formatResponseFlags(selectedSlowQuery.response_flags) }}</div>
            <div>
              <strong>应答详情:</strong>
              {{ selectedSlowQuery.answer_details_status || 'complete' }}
              <span v-if="selectedSlowQuery.answer_decode_error" class="muted">（{{ selectedSlowQuery.answer_decode_error }}）</span>
            </div>
            <div><strong>耗时:</strong> {{ formatDuration(selectedSlowQuery.duration_ms) }}</div>
          </div>
          <div class="table-wrap" style="margin-top: 10px;">
            <table>
              <thead>
                <tr>
                  <th>应答类型</th>
                  <th>数据</th>
                  <th class="text-right">TTL</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="!Array.isArray(selectedSlowQuery.answers) || selectedSlowQuery.answers.length === 0">
                  <td colspan="3" class="empty">(empty)</td>
                </tr>
                <tr v-for="(answer, index) in (selectedSlowQuery.answers || [])" :key="`slow-answer-${index}`">
                  <td>{{ answer.type || '-' }}</td>
                  <td class="mono">{{ answer.data || '-' }}</td>
                  <td class="text-right">{{ Number(answer.ttl || 0) }}s</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>
      </div>

      <div v-if="rankingDetail.open" class="modal-mask modal-mask-ranking" @click.self="closeRankingDetail">
        <section class="panel data-view-modal overview-ranking-detail-modal">
          <header class="panel-header">
            <div>
              <h3>{{ rankingDetail.title }}</h3>
              <p class="muted">{{ rankingDetail.headline || '-' }}</p>
              <p v-if="rankingDetail.subline" class="muted mono ranking-detail-subline">{{ rankingDetail.subline }}</p>
            </div>
            <div class="actions">
              <button class="btn secondary" type="button" @click="closeRankingDetail">关闭</button>
            </div>
          </header>
          <div class="overview-ranking-summary-grid">
            <article
              v-for="card in rankingDetailSummaryCards"
              :key="card.key"
              class="overview-ranking-summary-card"
            >
              <span class="overview-ranking-summary-label">{{ card.title }}</span>
              <strong>{{ card.value }}</strong>
            </article>
          </div>
          <div class="table-wrap ranking-detail-table-fit">
            <table>
              <thead>
                <tr>
                  <th>时间</th>
                  <th>域名</th>
                  <th>客户端</th>
                  <th>类型</th>
                  <th>分流规则</th>
                  <th>上游</th>
                  <th class="text-right">耗时</th>
                  <th>响应</th>
                  <th class="text-right">详情</th>
                </tr>
              </thead>
              <tbody>
              <tr v-if="rankingDetail.loading">
                <td colspan="9" class="empty">加载中...</td>
              </tr>
              <tr v-else-if="rankingDetail.logs.length === 0">
                <td colspan="9" class="empty">暂无详情数据</td>
              </tr>
              <tr v-for="(log, index) in rankingDetail.logs" :key="`ranking-log-${log.trace_id || log.query_time || index}`">
                <td data-label="时间">{{ formatTime(log.query_time) }}</td>
                <td data-label="域名" class="mono ranking-detail-domain-cell" :title="log.query_name || '-'">{{ log.query_name || '-' }}</td>
                <td data-label="客户端">{{ getClientDisplay(log.client_ip) }}</td>
                <td data-label="类型">{{ log.query_type || '-' }}</td>
                <td data-label="分流规则" :title="getDetailRuleLabel(getRuleDetailValue(log))">{{ getDetailRuleLabel(getRuleDetailValue(log)) }}</td>
                <td data-label="上游" :title="getUpstreamDetailValue(log)">{{ getUpstreamDetailValue(log) }}</td>
                <td data-label="耗时" class="text-right">{{ formatDuration(log.duration_ms) }}</td>
                <td data-label="响应">{{ log.response_code || '-' }}</td>
                <td data-label="详情" class="text-right">
                  <button class="btn tiny secondary" type="button" @click="openSlowDetail(log)">查看</button>
                </td>
              </tr>
              </tbody>
            </table>
          </div>
        </section>
      </div>
    </Teleport>
  </section>
</template>

<style scoped>
.upstream-stats-panel {
  padding-top: 10px;
}

.upstream-stats-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 8px;
}

.upstream-stats-title-wrap h3 {
  margin: 0;
}

.learner-summary {
  margin: 0 0 8px;
}

.upstream-stats-table {
  min-width: 380px;
  table-layout: fixed;
}

/* The four figures share the width evenly; the name column is a little wider. */
.upstream-stats-table th:nth-child(1) {
  width: 28%;
}

.upstream-stats-table th:nth-child(n + 2) {
  width: 18%;
}

.upstream-stats-group-row td,
.upstream-stats-group-row:hover td {
  padding-top: 16px;
  border-bottom: 0;
  background: transparent;
  color: var(--ink-0);
  font-size: 0.84rem;
  font-weight: 600;
}

.upstream-stats-group-count {
  margin-left: 6px;
  color: var(--ink-1);
  font-weight: 400;
}

.upstream-stats-name-cell {
  overflow: hidden;
  text-overflow: ellipsis;
}

.overview-unavailable {
  margin: 8px 12px 0;
  color: var(--muted);
  font-size: 12px;
}

.overview-unavailable span + span {
  margin-left: 12px;
}

@media (max-width: 760px) {
  .upstream-stats-head {
    align-items: flex-start;
    flex-direction: column;
  }
}
</style>
