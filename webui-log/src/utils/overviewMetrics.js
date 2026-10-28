// Display thresholds of the overview numbers (colour only, nothing else depends on them).
export const THRESHOLDS = Object.freeze({
  latencyMs: { good: 50, warn: 200 },       // <= 50 green, 50–200 yellow, > 200 red
  cacheHitPercent: { good: 90, warn: 70 },  // >= 90 green, 70–90 yellow, < 70 red
  cpuPercent: { warn: 50, bad: 90 },        // > 50 yellow, > 90 red, below: no colour
  memoryBytes: { warn: 128 * 1024 * 1024, bad: 256 * 1024 * 1024 }
})

const finite = (value) => typeof value === 'number' && Number.isFinite(value)

// Missing/invalid metadata is unavailable, never zero or a truncated rank size.
export function parseDistinctDomainCount(header) {
  if (typeof header !== 'string' || !/^\d+$/.test(header.trim())) return null
  const count = Number(header.trim())
  return Number.isSafeInteger(count) ? count : null
}

// 'ok' | 'warn' | 'bad' | '' (no colour / no value)
export function latencyTone(ms) {
  if (!finite(ms)) return ''
  if (ms <= THRESHOLDS.latencyMs.good) return 'ok'
  return ms <= THRESHOLDS.latencyMs.warn ? 'warn' : 'bad'
}

export function cacheHitTone(percent) {
  if (!finite(percent)) return ''
  if (percent >= THRESHOLDS.cacheHitPercent.good) return 'ok'
  return percent >= THRESHOLDS.cacheHitPercent.warn ? 'warn' : 'bad'
}

export function cpuTone(percent) {
  if (!finite(percent)) return ''
  if (percent > THRESHOLDS.cpuPercent.bad) return 'bad'
  return percent > THRESHOLDS.cpuPercent.warn ? 'warn' : ''
}

export function memoryTone(bytes) {
  if (!finite(bytes)) return ''
  if (bytes > THRESHOLDS.memoryBytes.bad) return 'bad'
  return bytes > THRESHOLDS.memoryBytes.warn ? 'warn' : ''
}

// One-core percentage between two samples `{cpuSeconds, at}` (`at` in milliseconds):
// CPU time used / wall time elapsed. null without a usable earlier sample (first refresh,
// clock going back, or a restart that reset the counter).
export function cpuPercent(previous, current) {
  if (!previous || !current) return null
  const wallSeconds = (current.at - previous.at) / 1000
  const used = current.cpuSeconds - previous.cpuSeconds
  if (!finite(wallSeconds) || !finite(used) || wallSeconds <= 0 || used < 0) return null
  return (used / wallSeconds) * 100
}

function metricValue(line, name) {
  if (!line.startsWith(`${name} `)) return null
  const value = Number(line.slice(name.length).trim().split(/\s+/)[0])
  return Number.isFinite(value) ? value : null
}

// The process figures of `/metrics` (null when absent).
export function parseProcessMetrics(text) {
  const out = { cpuSeconds: null, residentBytes: null, startTime: null, threads: null, openFds: null }
  const names = {
    cpuSeconds: 'process_cpu_seconds_total',
    residentBytes: 'process_resident_memory_bytes',
    startTime: 'process_start_time_seconds',
    threads: 'mosdns_process_threads',
    openFds: 'process_open_fds'
  }
  for (const line of String(text || '').split('\n')) {
    for (const [key, name] of Object.entries(names)) {
      const value = metricValue(line, name)
      if (value !== null) out[key] = value
    }
  }
  return out
}

// Hits / lookups summed over every named cache's counters; null without lookups.
export function cacheHitPercent(text) {
  let hits = 0
  let queries = 0
  for (const line of String(text || '').split('\n')) {
    const match = line.match(/^mosdns_cache_(hit_total|query_total)\{[^}]*\}\s+(\S+)/)
    if (!match) continue
    const value = Number(match[2])
    if (!Number.isFinite(value) || value < 0) continue
    if (match[1] === 'hit_total') hits += value
    else queries += value
  }
  return queries > 0 ? (hits / queries) * 100 : null
}

// "最近 X 小时 · N 条" from the oldest retained record and the time the windows were built.
export function coverageText(coverageStart, generatedAt, count) {
  const total = `${Math.round(Number(count) || 0).toLocaleString('zh-CN')} 条`
  const start = Date.parse(coverageStart || '')
  const end = Date.parse(generatedAt || '')
  if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) return `审计保留记录 · ${total}`
  const hours = (end - start) / 3_600_000
  const span = hours < 1 ? `${Math.max(1, Math.round(hours * 60))} 分钟` : hours < 48 ? `${hours.toFixed(1)} 小时` : `${(hours / 24).toFixed(1)} 天`
  return `最近 ${span} · ${total}`
}
