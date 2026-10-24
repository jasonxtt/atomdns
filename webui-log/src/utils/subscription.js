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

// Name and local file proposed for a new subscription from its address.
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
  return { name: fileName.replace(/\.[^.]+$/, '') || fileName, files: `srs/${fileName}` }
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
