// Helpers of the online-subscription table (china/non-china domains, china IPs, the two
// custom sets, AdGuard rules and the subscriptions of a special group).

// The five online sets of 分流规则: `type` is the key the rule records carry, `tag` the
// plugin that serves `/plugins/<tag>/config`.
export const ROUTING_SUBSCRIPTIONS = Object.freeze([
  { type: 'geositecn', tag: 'geosite_cn', label: '中国域名' },
  { type: 'geositenocn', tag: 'geosite_no_cn', label: '非中国域名' },
  { type: 'geoipcn', tag: 'geoip_cn', label: '中国 IP' },
  { type: 'cuscn', tag: 'cuscn', label: '直连补充 (cuscn)' },
  { type: 'cusnocn', tag: 'cusnocn', label: '代理补充 (cusnocn)' }
])

// The formats a subscription can be downloaded in; `auto` is not sent (the host detects it).
export const SUBSCRIPTION_FORMATS = Object.freeze([
  { value: 'auto', label: '自动' },
  { value: 'srs', label: 'SRS' },
  { value: 'text', label: '文本 (.txt .list .conf)' },
  { value: 'yaml', label: 'YAML' },
  { value: 'json', label: 'JSON' },
  { value: 'dat', label: 'DAT (v2ray geosite / geoip)' },
  { value: 'mrs', label: 'MRS (mihomo)' }
])

// A `.dat` file holds many categories, so the subscription names the one to import: with the
// DAT format, or with an automatic format and an address that ends in `.dat`.
export function needsCategory(format, url) {
  if (format === 'dat') return true
  if (format && format !== 'auto') return false
  const raw = String(url || '').trim()
  let path = raw.split('#')[0].split('?')[0]
  try {
    path = new URL(raw).pathname
  } catch {
    // keep the text before `?` and `#`
  }
  return /\.dat$/i.test(path)
}

// The record to save: the stored record's fields under the edited ones. `auto` leaves the
// field out, which also clears a format the record had before. The category is sent only
// where it is used and not empty, so a stored one goes when the format no longer needs it.
export function withFormat(base, payload, format, category = '') {
  const merged = { ...base, ...payload }
  if (!format || format === 'auto') delete merged.format
  else merged.format = format
  const text = String(category || '').trim()
  if (text && needsCategory(format, merged.url)) merged.category = text
  else delete merged.category
  return merged
}

// Name and local file proposed for a new subscription from its address. Only an `.srs` file
// is kept as it is; any other download is stored converted, as text.
export function inferFromUrl(url) {
  const raw = String(url || '').trim()
  if (!raw) return null
  let fileName = ''
  try {
    fileName = (new URL(raw).pathname || '').split('/').pop() || ''
  } catch {
    fileName = raw.split('#')[0].split('?')[0].split('/').pop() || ''
  }
  try {
    fileName = decodeURIComponent(fileName)
  } catch {
    // keep the raw name
  }
  fileName = fileName.trim()
  if (!fileName) return null
  const name = fileName.replace(/\.[^.]+$/, '') || fileName
  return { name, files: /\.srs$/i.test(fileName) ? `srs/${fileName}` : `srs/${name}.txt` }
}

function isZero(value) {
  return !value || String(value).startsWith('0001-01-01')
}

// A downloaded subscription has a count or an update time.
export function isDownloaded(rule) {
  return Number(rule?.rule_count || 0) > 0 || !isZero(rule?.last_updated)
}

// The fields a rule record keeps when it is saved back (the page's own `__` fields go).
export function ruleFields(rule) {
  return Object.fromEntries(Object.entries(rule || {}).filter(([key]) => !key.startsWith('__')))
}

export function sortRules(rules) {
  return [...rules].sort((a, b) => {
    const enabled = Number(Boolean(b.enabled)) - Number(Boolean(a.enabled))
    if (enabled !== 0) return enabled
    const type = String(a.__label || '').localeCompare(String(b.__label || ''))
    if (type !== 0) return type
    return String(a.name || '').localeCompare(String(b.name || ''))
  })
}
