// Helpers of the online-subscription table (the base sets, the two custom sets, AdGuard rules
// and the subscriptions of a special group).

// The five online sets of 分流规则, in two groups. `type` is the key the rule records carry,
// `tag` the plugin that serves `/plugins/<tag>/config`.
// 自定义订阅: the two sets a user adds to, edits and deletes freely.
export const CUSTOM_SUBSCRIPTIONS = Object.freeze([
  { type: 'cuscn', tag: 'cuscn', label: '直连域名' },
  { type: 'cusnocn', tag: 'cusnocn', label: '代理域名' }
])

// 基础规则: the sets that decide the China / non-China split. They can be turned off and
// pointed elsewhere but never deleted down to zero; `defaults` is the record of the shipped
// configuration package (srs/<type>.json).
export const BASE_SUBSCRIPTIONS = Object.freeze([
  {
    type: 'geositecn', tag: 'geosite_cn', label: '中国域名',
    defaults: { name: 'geosite_cn', files: 'srs/geosite-cn.srs', url: 'https://github.com/MetaCubeX/meta-rules-dat/raw/sing/geo/geosite/cn.srs' }
  },
  {
    type: 'geositenocn', tag: 'geosite_no_cn', label: '非中国域名',
    defaults: { name: 'geosite_no_cn', files: 'srs/geolocation-!cn.srs', url: 'https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/refs/heads/sing/geo/geosite/geolocation-!cn.srs' }
  },
  {
    type: 'geoipcn', tag: 'geoip_cn', label: '中国 IP',
    defaults: { name: 'geoip_cn', files: 'srs/geoip-cn.srs', url: 'https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/refs/heads/sing/geo/geoip/cn.srs' }
  }
])

export const ROUTING_SUBSCRIPTIONS = Object.freeze([...BASE_SUBSCRIPTIONS, ...CUSTOM_SUBSCRIPTIONS])

// What turning a base set off does, shown in the confirmation.
export const DISABLE_WARNINGS = Object.freeze({
  geoipcn: '关闭后，列表外域名查到的 IP 无法判断国内外，有 IP 的一律走代理。这是影响最大的一项。',
  geositecn: '关闭后，国内域名都要先查一次上游再判断，会变慢；非 A/AAAA 请求会全部走国外上游。',
  geositenocn: '关闭后，境外域名改为按中国 IP 判断，多一次上游查询。'
})

export function disableWarning(type) {
  return DISABLE_WARNINGS[type] || ''
}

// A base set keeps at least one source: its last one has no delete. Other tables delete freely.
export function canDeleteRule(rules, rule, variant) {
  if (variant !== 'base') return true
  return rules.filter((item) => item.type === rule.type).length > 1
}

// The catalogs of `loadedTypes` with no source at all: the base table shows a row to set each.
export function emptyCatalogs(rules, catalogs, loadedTypes) {
  return catalogs.filter((item) => loadedTypes.includes(item.type) && !rules.some((rule) => rule.type === item.type))
}

// The fields "恢复默认地址" puts back; the name and the local file stay.
export function defaultAddress(type) {
  const found = BASE_SUBSCRIPTIONS.find((item) => item.type === type)
  return found ? { url: found.defaults.url, format: 'auto', category: '' } : null
}

// Turning a base set off (a saved edit or a switch) is confirmed first.
export function needsDisableConfirm(variant, wasEnabled, willEnable) {
  return variant === 'base' && Boolean(wasEnabled) && !willEnable
}

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

// Name and local file proposed for a new subscription from its address. The file keeps the
// address's name when it has a rule-file extension (the program reads a file by its content,
// so a converted download may keep it); any other name gets `.txt`.
const RULE_FILE_EXTENSION = /\.(srs|mrs|dat|txt|list|conf|ya?ml|json)$/i

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
  return { name, files: RULE_FILE_EXTENSION.test(fileName) ? `srs/${fileName}` : `srs/${name}.txt` }
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
