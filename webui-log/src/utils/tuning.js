// Helpers for the tunable numbers of the cache and learning pages: the accepted ranges
// (the backend's, see native-settings.md), durations shown in the unit that fits and a
// check of one edited value.

export const CACHE_RANGES = Object.freeze({
  size: [1000, 100000000],
  lazy_cache_ttl: [0, 4294967295],
  dump_interval: [60, 31536000]
})

export const LEARNING_RANGES = Object.freeze({
  ttl: [60, 315360000],
  max_entries: [1, 262144],
  write_interval: [60, 31536000]
})

export const DURATION_UNITS = Object.freeze([
  { id: 'second', label: '秒', seconds: 1 },
  { id: 'minute', label: '分钟', seconds: 60 },
  { id: 'hour', label: '小时', seconds: 3600 },
  { id: 'day', label: '天', seconds: 86400 }
])

const YEAR = 365 * 86400
// Beyond this a duration reads better in years ("约 8.2 年"), e.g. the 3000-day retention.
const YEARS_FROM = 1000 * 86400

const unitById = (id) => DURATION_UNITS.find((unit) => unit.id === id) || DURATION_UNITS[0]
const trim = (value) => String(Math.round(value * 10) / 10)

// The largest unit that divides `seconds` evenly (days for 0), so 604800 is 7 days and
// 21600 is 6 hours; anything else is shown in seconds.
export function splitDuration(seconds) {
  const total = Number(seconds)
  if (!Number.isFinite(total) || total <= 0) return { value: 0, unit: 'day' }
  for (const unit of [...DURATION_UNITS].reverse()) {
    if (total >= unit.seconds && total % unit.seconds === 0) return { value: total / unit.seconds, unit: unit.id }
  }
  return { value: total, unit: 'second' }
}

// `NaN` when the amount is not a number, so the range check rejects it.
export function toSeconds(value, unit) {
  const amount = typeof value === 'string' && value.trim() === '' ? Number.NaN : Number(value)
  return Math.round(amount * unitById(unit).seconds)
}

export function formatDuration(seconds) {
  const total = Number(seconds)
  if (!Number.isFinite(total)) return '—'
  if (total >= YEARS_FROM) return `约 ${trim(total / YEAR)} 年`
  for (const unit of [...DURATION_UNITS].reverse()) {
    if (total >= unit.seconds) return `${trim(total / unit.seconds)} ${unit.label}`
  }
  return `${total} 秒`
}

export function formatCount(value) {
  return Number.isFinite(Number(value)) ? Number(value).toLocaleString('en-US') : '—'
}

// '' when `value` is a whole number inside `range`; otherwise what is wrong.
// `kind` is 'count' or 'duration' and only picks how the range is written.
export function fieldError(value, range, label, kind) {
  const [low, high] = range
  const show = kind === 'duration' ? formatDuration : formatCount
  if (typeof value !== 'number' || !Number.isInteger(value)) return `${label}需要是整数`
  if (value < low || value > high) return `${label}需在 ${show(low)} 到 ${show(high)} 之间`
  return ''
}
