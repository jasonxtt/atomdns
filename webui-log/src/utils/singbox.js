// Pure helpers for sing-box subscription and node management.
export function validateSubscription(form) {
  const name = String(form?.name || '').trim()
  const url = String(form?.url || '').trim()
  if (!name || name.length > 255 || /[\s/]/.test(name)) return '订阅名称不能为空，且不能包含空格或 /'
  if (!/^https?:\/\/[^\s]+$/i.test(url)) return '订阅地址必须是 http:// 或 https:// URL'
  const hours = Number(form?.update_interval_hours)
  if (!Number.isInteger(hours) || hours < 1 || hours > 8760) return '更新间隔必须是 1–8760 小时的整数'
  return ''
}

export function validateNode(form) {
  const link = String(form?.link || '').trim()
  const tag = String(form?.tag || '').trim()
  if (!link && form?.editing) return tag ? '' : '节点标识不能为空'
  if (!link || /[\r\n]/.test(link)) return '请粘贴一条节点链接'
  if (!/^[a-z][a-z0-9+.-]*:\/\/\S+$/i.test(link)) return '节点链接格式不正确，例如 socks5h://… 或 vless://…'
  if (tag && (tag.length > 255 || /[\s/]/.test(tag))) return '节点标识不能为空，且不能包含空格或 /'
  return ''
}

export function nodeSource(name) {
  const text = String(name || '')
  return text.includes('/') ? text.split('/')[0] : '手动节点'
}

export function latestDelay(node) {
  const delay = Number(node?.delay)
  return Number.isFinite(delay) && delay > 0 ? `${delay} ms` : '未测速'
}

export function groupByName(groups, name) {
  return (Array.isArray(groups) ? groups : []).find((group) => group.name === name) || null
}
