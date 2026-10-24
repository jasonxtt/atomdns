// Client aliases (`/plugins/clientname`): an address → name map the log pages use.

export function normalizeIP(ip) {
  return String(ip || '').replace(/^::ffff:/, '').trim()
}

export function normalizeAliasMap(input) {
  const normalized = {}
  if (!input || typeof input !== 'object' || Array.isArray(input)) return normalized
  for (const [ip, alias] of Object.entries(input)) {
    const key = normalizeIP(ip)
    const value = String(alias || '').trim()
    if (key && value) normalized[key] = value
  }
  return normalized
}

const collator = new Intl.Collator('zh-CN', { numeric: true, sensitivity: 'base' })

export function sortAliasRows(rows) {
  return [...rows].sort((a, b) => collator.compare(a.ip, b.ip))
}

// The editable rows: every saved alias plus the addresses seen lately, by address.
export function aliasRowsFrom(map, knownIPs = []) {
  const unique = new Set([...Object.keys(map || {}), ...knownIPs.map(normalizeIP).filter(Boolean)])
  return sortAliasRows([...unique].map((ip) => ({ ip, alias: String(map?.[ip] || ''), originalAlias: String(map?.[ip] || '') })))
}

// What the rows save: rows with an address and a name; an empty name removes the alias.
export function aliasMapFromRows(rows) {
  const map = {}
  for (const row of rows) {
    const ip = normalizeIP(row.ip)
    const alias = String(row.alias || '').trim()
    if (ip && alias) map[ip] = alias
  }
  return map
}

export function aliasesChanged(rows) {
  return rows.some((row) => String(row.alias || '').trim() !== String(row.originalAlias || '').trim())
}
