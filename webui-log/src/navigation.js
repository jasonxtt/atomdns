// The sidebar and the routes come from this one table: four modules, the sub-items of
// 规则设置 expanded in the sidebar (`#/rules/<id>`). 日志 has a third-level page
// (`#/logs/diagnostic`) and 系统管理 is one page (`#/system`) with sections.

export const RULE_SECTIONS = Object.freeze([
  { id: 'mode', label: '运行模式' },
  { id: 'upstream', label: '上游设置' },
  { id: 'routing', label: '分流规则' },
  { id: 'blocking', label: '请求屏蔽' },
  { id: 'rewrite', label: '改写规则' },
  { id: 'learning', label: '域名学习' },
  { id: 'cache', label: '缓存设置' },
  { id: 'clients', label: '客户端' },
  { id: 'special-groups', label: '专属分流组' }
])

export const NAV_MODULES = Object.freeze([
  { id: 'overview', label: '概览', path: '/overview', items: [] },
  { id: 'logs', label: '日志', path: '/logs', items: [] },
  { id: 'rules', label: '规则设置', path: `/rules/${RULE_SECTIONS[0].id}`, items: RULE_SECTIONS.map(item => ({ ...item, path: `/rules/${item.id}` })) },
  { id: 'system', label: '系统管理', path: '/system', items: [] }
])

// Every page the router knows: `name` is the key of the page component map.
export const PAGES = Object.freeze([
  { name: 'overview', path: '/overview', title: '概览' },
  { name: 'logs', path: '/logs', title: '日志' },
  { name: 'logs-diagnostic', path: '/logs/diagnostic', title: '诊断抓取' },
  ...RULE_SECTIONS.map(item => ({ name: `rules-${item.id}`, path: `/rules/${item.id}`, title: item.label })),
  { name: 'system', path: '/system', title: '系统管理' }
])
