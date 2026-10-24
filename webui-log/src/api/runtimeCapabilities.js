import { reactive } from 'vue'

export const OPERATION_IDS = Object.freeze([
  'system.health', 'system.version', 'audit.read', 'audit.control', 'audit.capacity',
  'query.rank', 'cache.inventory', 'cache.manage', 'metrics.cache', 'rules.local.read',
  'rules.local.manage', 'groups.read', 'groups.manage', 'groups.upstreams', 'upstreams.read', 'upstreams.manage',
  'rules.diversion', 'rules.adguard', 'capture.logs', 'client.aliases', 'flags.manage',
  'cache.requery', 'lists.remembered', 'system.restart', 'system.webui_port',
  'system.config_management', 'system.update', 'system.domain_generation', 'system.global_overrides', 'metrics.process'
])
function isObject(value) { return value !== null && typeof value === 'object' && !Array.isArray(value) }

export function validateRuntimeCapabilities(value) {
  if (!isObject(value) || value.schema_version !== 1 || value.runtime !== 'rust' || !isObject(value.special_groups) || typeof value.special_groups.enabled !== 'boolean' || !Array.isArray(value.upstream_protocols) || !value.upstream_protocols.every(item => typeof item === 'string') || !Array.isArray(value.rule_formats) || !value.rule_formats.every(item => typeof item === 'string') || !Array.isArray(value.unsupported_features) || !value.unsupported_features.every(item => typeof item === 'string') || !isObject(value.endpoints)) throw new Error('invalid runtime capability response')
  if (!isObject(value.ui_operations) || OPERATION_IDS.some(id => {
    const op = value.ui_operations[id]
    return !isObject(op) || typeof op.supported !== 'boolean' || (op.supported ? op.reason !== null : typeof op.reason !== 'string' || !op.reason.trim())
  })) throw new Error('invalid runtime operation capability response')
  return {...value, ui_operations: {...value.ui_operations}}
}
async function discover(url) {
  // Discovery alone bypasses operation admission, avoiding a dependency cycle.
  const response = await fetch(url)
  if (!response.ok) throw Object.assign(new Error(`HTTP ${response.status}`), {status:response.status})
  return response.json()
}
export function createRuntimeCapabilityClient(request = discover) {
  let pending
  const get = () => {
    // `kind` marks the runtime for the helpers that check it (settingsBackedProxy).
    if (!pending) pending = Promise.resolve().then(() => request('/api/v1/capabilities'))
      .then(response => ({...validateRuntimeCapabilities(response), kind:'native'}))
      .catch(error => { pending = null; throw error })
    return pending
  }
  get.invalidate = () => { pending = null }
  return get
}
const client = createRuntimeCapabilityClient()
export const capabilityState = reactive({status:'pending', value:null, error:'', health:null, cacheTags:new Set()})
let loading
export function getRuntimeCapabilities() {
  if (capabilityState.status === 'ready') return Promise.resolve(capabilityState.value)
  if (capabilityState.status === 'error') return Promise.reject(new Error(capabilityState.error))
  if (!loading) {
    capabilityState.status = 'pending'
    loading = client().then(value => {capabilityState.value = value; capabilityState.status = 'ready'; capabilityState.error = ''; return value})
      .catch(error => {capabilityState.status = 'error'; capabilityState.error = error.message; throw error})
      .finally(() => {loading = null})
  }
  return loading
}
export async function refreshRuntimeCapabilities() { client.invalidate(); capabilityState.status = 'pending'; return getRuntimeCapabilities() }
export function supportsOperation(id) {
  if (capabilityState.status !== 'ready') return false
  return capabilityState.value?.ui_operations?.[id]?.supported === true
}
export function operationReason(id) {
  if (capabilityState.status === 'pending') return '正在读取运行时能力'
  if (capabilityState.status === 'error') return `运行时能力读取失败：${capabilityState.error}`
  return supportsOperation(id) ? '' : capabilityState.value?.ui_operations?.[id]?.reason || '后端未声明此能力'
}
export async function retryRuntimeCapabilities() {
  await refreshRuntimeCapabilities()
  if (typeof window !== 'undefined') { window.dispatchEvent(new CustomEvent('mosdns-capabilities-ready')); window.dispatchEvent(new CustomEvent('mosdns-log-refresh')) }
}

// One endpoint inventory for shared HTTP, raw upload/export calls and services.
export function requestOperation(url, method = 'GET') {
  const path = new URL(url, 'http://capability.invalid').pathname
  if (path === '/api/v1/capabilities') return null
  if (path === '/api/v1/system/health') return 'system.health'
  if (path.startsWith('/api/v1/capture/')) return 'capture.logs'
  if (path.startsWith('/api/v1/update/')) return 'system.update'
  if (path.startsWith('/api/v1/config/')) return 'system.config_management'
  if (path === '/api/v1/system/restart') return 'system.restart'
  if (path === '/api/v1/system/webui-port') return 'system.webui_port'
  if (path.startsWith('/api/v1/domain-generation')) return 'system.domain_generation'
  if (path.startsWith('/api/v1/overrides')) return 'system.global_overrides'
  if (path === '/api/v1/flags' || path.startsWith('/api/v1/flags/')) return 'flags.manage'
  if (/^\/plugins\/clientname(?:\/|$)/.test(path)) return 'client.aliases'
  if (/^\/plugins\/adguard\/(rules|update)(?:\/|$)/.test(path)) return 'rules.adguard'
  if (/^\/plugins\/requery(?:\/|$)/.test(path)) return 'cache.requery'
  if (/^\/plugins\/route_learner\/(status|entries)$/.test(path)) return 'lists.remembered'
  if (/^\/plugins\/(my_[^/]+|top_domains)(?:\/|$)/.test(path)) return 'lists.remembered'
  if (/^\/api\/v[12]\/audit\/(capacity|settings)$/.test(path)) return 'audit.capacity'
  if (/^\/api\/v[12]\/audit\/(start|stop|enable|disable|clear)$/.test(path)) return 'audit.control'
  if (/^\/api\/v2\/audit\/rank\//.test(path)) return 'query.rank'
  if (/^\/api\/v[12]\/audit\//.test(path)) return 'audit.read'
  if (path === '/api/v1/cache/inventory') return 'cache.inventory'
  if (path === '/api/v1/upstream/sources') return 'groups.upstreams'
  if (path.startsWith('/api/v1/special-groups')) return method === 'GET' ? 'groups.read' : 'groups.manage'
  if (path.startsWith('/api/v1/upstream/')) return method === 'GET' ? 'upstreams.read' : 'upstreams.manage'
  if (path === '/metrics') return 'metrics.cache'
  if (/^\/plugins\/[^/]+\/(config|update)(?:\/|$)/.test(path)) return 'rules.diversion'
  if (/^\/plugins\/[^/]+\/(show|dump|save|flush|load_dump|search)$/.test(path)) {
    const tag = path.split('/')[2]
    return !/^cache/.test(tag) && !capabilityState.cacheTags.has(decodeURIComponent(tag)) && /\/(show|save)$/.test(path) ? (path.endsWith('/show') ? 'rules.local.read' : 'rules.local.manage') : 'cache.manage'
  }
  if (/^\/plugins\/[^/]+\/post$/.test(path)) return 'rules.local.manage'
  return null
}
export async function capabilityFetch(url, options = {}) {
  const method = (options.method || 'GET').toUpperCase()
  let operation = requestOperation(url, method)
  if (operation) {
    await getRuntimeCapabilities()
    operation = requestOperation(url, method)
    if (!supportsOperation(operation)) throw Object.assign(new Error(operationReason(operation)), {operation, capabilityDisabled:true})
  }
  const response = await fetch(url, options)
  if (response.ok && operation === 'cache.inventory') { const inventory = await response.clone().json(); capabilityState.cacheTags = new Set((inventory.caches || []).map(cache => cache.tag)) }
  if (response.ok && operation === 'system.health') capabilityState.health = await response.clone().json()
  if (response.ok && method !== 'GET' && ['groups.manage','upstreams.manage','rules.diversion','rules.local.manage'].includes(operation)) {
    try { await refreshRuntimeCapabilities() } catch { /* Successful mutation ACK stays factual; shared discovery error disables subsequent optional work and exposes retry. */ }
  }
  return response
}
