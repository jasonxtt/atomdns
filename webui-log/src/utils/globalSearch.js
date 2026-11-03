// Pure helpers of the global search palette (Ctrl/Cmd+K): ranking, address matching and the
// result entries of each source. Every entry is `{ id, kind, title, detail, route }`; `route`
// is a vue-router location whose query the target page reads to scroll to / filter the hit.

export const SEARCH_KINDS = Object.freeze([
  { kind: 'page', label: '页面' },
  { kind: 'flag', label: '设置项' },
  { kind: 'client', label: '客户端' },
  { kind: 'domain', label: '域名名单' },
  { kind: 'iplist', label: 'IP 名单' },
  { kind: 'group', label: '专属分流组' },
  { kind: 'log', label: '查询日志' }
])

export const SECTION_LIMIT = 6
export const EMPTY_LIMIT = 5

// The manual lists the pages edit: `tag` → the sub-item that shows it. `ip` lists hold
// addresses, networks or ranges; the others hold domain rules (or rewrite pairs).
export const SEARCH_LISTS = Object.freeze([
  { tag: 'whitelist', title: '白名单', page: 'routing', tab: 'lists' },
  { tag: 'greylist', title: '灰名单', page: 'routing', tab: 'lists' },
  { tag: 'realiplist', title: '!CN fakeip filter', page: 'routing', tab: 'fakeip' },
  { tag: 'cnfakeipfilter', title: 'CN fakeip filter', page: 'routing', tab: 'fakeip' },
  { tag: 'direct_ip', title: '直连 IP', page: 'routing', tab: 'ip-rewrite', ip: true },
  { tag: 'rewrite', title: '重定向', page: 'routing', tab: 'ip-rewrite' },
  { tag: 'ddnslist', title: 'DDNS 域名', page: 'routing', tab: 'ddns' },
  { tag: 'blocklist', title: '黑名单', page: 'blocking' },
  { tag: 'client_ip', title: '客户端 IP', page: 'clients', ip: true }
])

export function normalizeQuery(text) {
  return String(text || '').trim().toLowerCase().replace(/\s+/g, ' ')
}

// 0 exact, 1 prefix, 2 contains, 3 no match.
export function rank(needle, keys) {
  let best = 3
  for (const key of keys) {
    const text = String(key || '').toLowerCase()
    if (!text) continue
    if (text === needle) return 0
    if (text.startsWith(needle)) best = Math.min(best, 1)
    else if (text.includes(needle)) best = Math.min(best, 2)
  }
  return best
}

// Keeps the entries that match, best first and in source order within a rank.
export function pick(entries, needle, limit = SECTION_LIMIT) {
  const scored = []
  entries.forEach((entry, index) => {
    const score = needle ? rank(needle, entry.keys || [entry.title]) : 0
    if (score < 3) scored.push({ entry, score, index })
  })
  scored.sort((a, b) => a.score - b.score || a.index - b.index)
  return scored.slice(0, limit).map((item) => item.entry)
}

// ----- addresses ------------------------------------------------------------------------

export function parseIP(text) {
  const value = String(text || '').trim()
  const v4 = value.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/)
  if (v4) {
    const parts = v4.slice(1).map(Number)
    if (parts.some((part) => part > 255)) return null
    return { v: 4, n: parts.reduce((total, part) => (total << 8n) | BigInt(part), 0n) }
  }
  if (!/^[0-9a-f:]+$/i.test(value) || !value.includes(':') || (value.match(/::/g) || []).length > 1) return null
  const [head, tail] = value.split('::')
  const headParts = head ? head.split(':') : []
  const tailParts = tail === undefined ? [] : tail ? tail.split(':') : []
  const missing = 8 - headParts.length - tailParts.length
  if (tail === undefined ? missing !== 0 : missing < 1) return null
  const groups = [...headParts, ...Array(tail === undefined ? 0 : missing).fill('0'), ...tailParts]
  if (groups.length !== 8 || groups.some((group) => !/^[0-9a-f]{1,4}$/i.test(group))) return null
  return { v: 6, n: groups.reduce((total, group) => (total << 16n) | BigInt(parseInt(group, 16)), 0n) }
}

// Whether one list line (address, `addr/prefix` or `from-to` range) covers `ip`.
export function lineCoversIP(line, ip) {
  const target = typeof ip === 'string' ? parseIP(ip) : ip
  if (!target) return false
  const text = String(line || '').trim()
  const range = text.match(/^(\S+?)\s*-\s*(\S+)$/)
  if (range) {
    const [from, to] = [parseIP(range[1]), parseIP(range[2])]
    return Boolean(from && to && from.v === target.v && to.v === target.v && from.n <= target.n && target.n <= to.n)
  }
  const [addr, prefix] = text.split('/')
  const base = parseIP(addr)
  if (!base || base.v !== target.v) return false
  if (prefix === undefined) return base.n === target.n
  const bits = target.v === 4 ? 32 : 128
  const length = Number(prefix)
  if (!Number.isInteger(length) || length < 0 || length > bits) return false
  const shift = BigInt(bits - length)
  return base.n >> shift === target.n >> shift
}

// ----- entries --------------------------------------------------------------------------

const RULE_PAGE_TITLE = Object.freeze({ mode: '运行模式', upstream: '上游设置', routing: '分流规则', blocking: '请求屏蔽', learning: '域名学习', cache: '缓存设置', clients: '客户端', 'special-groups': '专属分流组' })

export function pageTitleOf(page) {
  return RULE_PAGE_TITLE[page] || page
}

export function flagEntries(flags, pageOfFlag) {
  return (Array.isArray(flags) ? flags : []).map((flag) => {
    const page = pageOfFlag(flag)
    const label = String(flag.label || flag.key)
    const options = Array.isArray(flag.options) ? flag.options.map((option) => option.label || option.value) : []
    return {
      id: `flag:${flag.key}`,
      kind: 'flag',
      title: label,
      detail: pageTitleOf(page),
      keys: [label, flag.key, String(flag.description || ''), ...options],
      route: { path: `/rules/${page}`, query: { flag: flag.key } }
    }
  }).filter((entry) => entry.route.path !== '/rules/')
}

export function groupEntries(groups) {
  return (Array.isArray(groups) ? groups : []).map((group) => ({
    id: `group:${group.slot}`,
    kind: 'group',
    title: String(group.name || `组 ${group.slot}`),
    detail: group.listen_port ? `端口 ${group.listen_port}` : '专属分流组',
    keys: [group.name, group.listen_port ? String(group.listen_port) : ''],
    route: { path: '/rules/special-groups', query: { group: String(group.slot) } }
  }))
}

// Client aliases `{ ip: name }`: found by the name or by an address (also by a network or range
// that is not listed: the log page lists those clients by address).
export function aliasEntries(aliases) {
  return Object.entries(aliases || {}).map(([ip, name]) => ({
    id: `client:${ip}`,
    kind: 'client',
    title: String(name || ip),
    detail: ip,
    keys: [name, ip],
    route: { path: '/rules/clients', query: { client: ip } }
  }))
}

// Whether a domain rule (`full:`, `domain:`, `keyword:`, `regexp:`, or a bare suffix; a rewrite
// line `domain target` counts by its first word) covers `domain`.
export function ruleCoversDomain(line, domain) {
  const name = String(domain || '').toLowerCase().replace(/\.$/, '')
  const first = String(line || '').split('#')[0].trim().split(/\s+/)[0] || ''
  if (!name || !first) return false
  const [, kind, value] = first.match(/^(full|domain|keyword|regexp):(.+)$/i) || [null, 'domain', first]
  const rule = value.toLowerCase()
  switch (kind.toLowerCase()) {
    case 'full': return name === rule
    case 'keyword': return name.includes(rule)
    case 'regexp':
      try { return new RegExp(value, 'i').test(name) } catch { return false }
    default: return name === rule || name.endsWith(`.${rule}`)
  }
}

// Lines of one list that match `query`: by text, or, for an address list, by covering the
// address typed, or, for a domain list, by a rule that covers the domain typed. Returns `{ lines, total }` with at most `limit` lines.
export function listMatches(text, query, { ip = false, limit = 3 } = {}) {
  const needle = normalizeQuery(query)
  if (!needle) return { lines: [], total: 0 }
  const target = ip ? parseIP(needle) : null
  const domain = !ip && /^[a-z0-9_-]+(\.[a-z0-9_-]+)+\.?$/.test(needle) && !parseIP(needle)
  const lines = []
  let total = 0
  for (const raw of String(text || '').split('\n')) {
    const line = raw.trim()
    if (!line) continue
    if (line.toLowerCase().includes(needle) || (target && lineCoversIP(line, target)) || (domain && ruleCoversDomain(line, needle))) {
      total += 1
      if (lines.length < limit) lines.push(line)
    }
  }
  return { lines, total }
}

export function listEntries(list, text, query, route) {
  const { lines, total } = listMatches(text, query, { ip: Boolean(list.ip) })
  return lines.map((line, index) => ({
    id: `list:${list.tag}:${line}`,
    kind: list.ip ? 'iplist' : 'domain',
    title: line,
    detail: total > lines.length && index === lines.length - 1 ? `${list.title} · 共 ${total} 条匹配` : list.title,
    route: route(list, line)
  }))
}

// A log hit: the domain asked and who asked.
export function logEntry(log, aliases = {}) {
  const domain = String(log.query_name || log.domain || '')
  const client = String(log.client_ip || '')
  return {
    id: `log:${domain}:${client}:${log.query_time || log.time || ''}:${log.query_type || ''}`,
    kind: 'log',
    title: domain || client,
    detail: [aliases[client] || client, log.query_type].filter(Boolean).join(' · '),
    route: { path: '/logs', query: { q: domain || client } }
  }
}

// Whether the text looks like something to look up in the logs (a domain or an address).
export function looksLikeLookup(needle) {
  return needle.length >= 2 && /^[\w.*:-]+$/.test(needle)
}
