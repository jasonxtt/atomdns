// The sidebar and the routes come from this one table: four modules, the sub-items of
// 规则设置 (`#/rules/<id>`) listed under it in the sidebar, collapsed until opened. 日志 has a third-level page
// (`#/logs/diagnostic`) and 系统管理 is one page (`#/system`) with sections.

export const RULE_SECTIONS = Object.freeze([
  { id: 'mode', label: '运行模式', icon: 'shuffle' },
  { id: 'upstream', label: '上游设置', icon: 'globe' },
  { id: 'routing', label: '分流规则', icon: 'route' },
  { id: 'blocking', label: '请求屏蔽', icon: 'ban' },
  { id: 'learning', label: '域名学习', icon: 'book' },
  { id: 'cache', label: '缓存设置', icon: 'database' },
  { id: 'clients', label: '客户端', icon: 'users' },
  { id: 'special-groups', label: '专属分流组', icon: 'layers' }
])

// Former sub-items that moved: address → where it is now.
export const REDIRECTS = Object.freeze([
  { from: '/rules/rewrite', to: '/rules/routing' }
])

export const NAV_MODULES = Object.freeze([
  { id: 'overview', label: '概览', icon: 'gauge', path: '/overview', items: [] },
  { id: 'logs', label: '日志', icon: 'list', path: '/logs', items: [] },
  { id: 'rules', label: '规则设置', icon: 'sliders', path: `/rules/${RULE_SECTIONS[0].id}`, items: RULE_SECTIONS.map(item => ({ ...item, path: `/rules/${item.id}` })) },
  { id: 'system', label: '系统管理', icon: 'settings', path: '/system', items: [] }
])

// Every page the router knows: `name` is the key of the page component map.
export const PAGES = Object.freeze([
  { name: 'overview', path: '/overview', title: '概览' },
  { name: 'logs', path: '/logs', title: '日志' },
  { name: 'logs-diagnostic', path: '/logs/diagnostic', title: '诊断抓取' },
  ...RULE_SECTIONS.map(item => ({ name: `rules-${item.id}`, path: `/rules/${item.id}`, title: item.label })),
  { name: 'system', path: '/system', title: '系统管理' }
])

// A module owns `/<id>` and everything below it.
export const inModule = (id, path) => path === `/${id}` || String(path || '').startsWith(`/${id}/`)

// Whether a module's sub-items are shown after a route change: clicking the module only opens
// or closes the list (`expanded`); arriving on one of its pages opens it, leaving keeps the choice.
export function expandedAfterRoute(id, path, expanded) {
  return inModule(id, path) ? true : Boolean(expanded)
}
