import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { createMemoryHistory } from 'vue-router'

import { NAV_MODULES, PAGES, RULE_SECTIONS } from '../src/navigation.js'
import { createAppRouter } from '../src/router.js'
import { THEME_STORAGE_KEY, createThemeController, normalizeThemeMode, resolveTheme } from '../src/utils/theme.js'
import {
  THRESHOLDS,
  cacheHitPercent,
  cacheHitTone,
  coverageText,
  cpuPercent,
  cpuTone,
  latencyTone,
  memoryTone,
  parseProcessMetrics
} from '../src/utils/overviewMetrics.js'

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')

async function routerAt(path) {
  const router = createAppRouter({ history: createMemoryHistory(), pageComponent: (name) => ({ name, render: () => null }) })
  await router.push(path)
  return router
}

test('the four modules carry the eight rule sub-items; logs has a diagnostic page; system is one page', () => {
  assert.deepEqual(NAV_MODULES.map((module) => module.label), ['概览', '日志', '规则设置', '系统管理'])
  assert.deepEqual(RULE_SECTIONS.map((item) => item.label), ['运行模式', '上游设置', '分流规则', '请求屏蔽', '域名学习', '缓存设置', '客户端', '专属分流组'])
  assert.deepEqual(NAV_MODULES.filter((module) => module.items.length > 0).map((module) => module.id), ['rules'])
  assert.deepEqual(PAGES.filter((page) => /^\/(logs|system)/.test(page.path)).map((page) => page.path), ['/logs', '/logs/diagnostic', '/system'])
  assert.equal(PAGES.length, 3 + 8 + 1)
  assert.equal(new Set(PAGES.map((page) => page.path)).size, PAGES.length)
})

test('every page address resolves to its own route, so a reload stays on the page', async () => {
  for (const page of PAGES) {
    const router = await routerAt(page.path)
    assert.equal(router.currentRoute.value.name, page.name, page.path)
    assert.equal(router.currentRoute.value.path, page.path)
  }
})

test('the root, module roots and unknown addresses redirect', async () => {
  assert.equal((await routerAt('/')).currentRoute.value.path, '/overview')
  assert.equal((await routerAt('/rules')).currentRoute.value.path, '/rules/mode')
  assert.equal((await routerAt('/system')).currentRoute.value.path, '/system')
  for (const old of ['/system/update', '/system/backup', '/system/audit', '/system/ui', '/system/service', '/system/a/b']) {
    assert.equal((await routerAt(old)).currentRoute.value.path, '/system', old)
  }
  assert.equal((await routerAt('/logs/diagnostic')).currentRoute.value.name, 'logs-diagnostic')
  // The former 改写规则 sub-item is part of 分流规则 now.
  assert.equal((await routerAt('/rules/rewrite')).currentRoute.value.path, '/rules/routing')
  for (const unknown of ['/nope', '/rules/nope', '/logs/nope', '/log']) {
    assert.equal((await routerAt(unknown)).currentRoute.value.path, '/overview', unknown)
  }
})

test('the app uses hash history and the static page map covers every route', () => {
  assert.match(read('../src/router.js'), /createWebHashHistory\(\)/)
  const pages = read('../src/pages/index.js')
  for (const page of PAGES) assert.match(pages, new RegExp(`['\\s]${page.name}'?:`), page.name)
  assert.doesNotMatch(pages, /import\(/, 'pages are bundled into app.js, no lazy chunks')
})

test('theme modes resolve to light or dark, defaulting to the system', () => {
  assert.equal(normalizeThemeMode('dark'), 'dark')
  assert.equal(normalizeThemeMode('sakura'), 'system')
  assert.equal(normalizeThemeMode(null), 'system')
  assert.equal(resolveTheme('system', true), 'dark')
  assert.equal(resolveTheme('system', false), 'light')
  assert.equal(resolveTheme('light', true), 'light')
  assert.equal(resolveTheme('dark', false), 'dark')
})

test('the theme controller applies, follows the system only in system mode, and saves the choice', () => {
  const attributes = {}
  const root = { setAttribute: (name, value) => { attributes[name] = value } }
  const store = new Map()
  const storage = { getItem: (key) => store.get(key) ?? null, setItem: (key, value) => store.set(key, value) }
  let listener = null
  const query = { matches: false, addEventListener: (_event, fn) => { listener = fn } }
  const controller = createThemeController({ root, storage, matchMedia: () => query })
  assert.equal(controller.mode, 'system')
  assert.equal(attributes['data-theme'], 'light')
  query.matches = true
  listener()
  assert.equal(attributes['data-theme'], 'dark', 'follows the system')
  controller.setMode('light')
  assert.equal(attributes['data-theme'], 'light')
  assert.equal(store.get(THEME_STORAGE_KEY), 'light')
  listener()
  assert.equal(attributes['data-theme'], 'light', 'a fixed mode ignores the system')
  // A stored mode is used on the next page load; a throwing storage falls back to the system.
  assert.equal(createThemeController({ root, storage, matchMedia: () => query }).mode, 'light')
  const broken = { getItem: () => { throw new Error('denied') }, setItem: () => { throw new Error('denied') } }
  const fallback = createThemeController({ root, storage: broken, matchMedia: () => query })
  assert.equal(fallback.mode, 'system')
  assert.equal(fallback.setMode('dark'), 'dark')
})

test('the colour thresholds follow the decided boundaries', () => {
  assert.deepEqual([latencyTone(0), latencyTone(50), latencyTone(50.1), latencyTone(200), latencyTone(200.1)], ['ok', 'ok', 'warn', 'warn', 'bad'])
  assert.deepEqual([cacheHitTone(100), cacheHitTone(90), cacheHitTone(89.9), cacheHitTone(70), cacheHitTone(69.9)], ['ok', 'ok', 'warn', 'warn', 'bad'])
  assert.deepEqual([cpuTone(10), cpuTone(50), cpuTone(50.1), cpuTone(90), cpuTone(90.1)], ['', '', 'warn', 'warn', 'bad'])
  const MiB = 1024 * 1024
  assert.deepEqual([memoryTone(100 * MiB), memoryTone(128 * MiB), memoryTone(129 * MiB), memoryTone(256 * MiB), memoryTone(257 * MiB)], ['', '', 'warn', 'warn', 'bad'])
  for (const tone of [latencyTone, cacheHitTone, cpuTone, memoryTone]) {
    assert.equal(tone(null), '')
    assert.equal(tone(Number.NaN), '')
  }
  assert.equal(THRESHOLDS.memoryBytes.warn, 128 * MiB)
})

test('CPU percent is CPU seconds over wall seconds between two samples, one core = 100%', () => {
  assert.equal(cpuPercent({ cpuSeconds: 10, at: 0 }, { cpuSeconds: 10.5, at: 5000 }), 10)
  assert.equal(cpuPercent({ cpuSeconds: 1, at: 1000 }, { cpuSeconds: 4, at: 3000 }), 150, 'more than one core can exceed 100%')
  assert.equal(cpuPercent(null, { cpuSeconds: 1, at: 1 }), null, 'the first sample has no figure')
  assert.equal(cpuPercent({ cpuSeconds: 5, at: 2000 }, { cpuSeconds: 6, at: 2000 }), null, 'no elapsed time')
  assert.equal(cpuPercent({ cpuSeconds: 9, at: 0 }, { cpuSeconds: 1, at: 1000 }), null, 'a restart reset the counter')
})

test('the process and cache figures are read from the metrics text', () => {
  const text = [
    '# TYPE mosdns_cache_query_total counter',
    'mosdns_cache_query_total{tag="cache_a"} 300',
    'mosdns_cache_query_total{tag="cache_b"} 100',
    'mosdns_cache_hit_total{tag="cache_a"} 270',
    'mosdns_cache_hit_total{tag="cache_b"} 50',
    'mosdns_cache_lazy_hit_total{tag="cache_a"} 7',
    'process_cpu_seconds_total 12.5',
    'process_resident_memory_bytes 52428800',
    'process_start_time_seconds 1700000000',
    'process_open_fds 31',
    'mosdns_process_threads 6'
  ].join('\n')
  assert.equal(cacheHitPercent(text), 80)
  assert.equal(cacheHitPercent('mosdns_cache_query_total{tag="a"} 0'), null)
  assert.deepEqual(parseProcessMetrics(text), { cpuSeconds: 12.5, residentBytes: 52428800, startTime: 1700000000, threads: 6, openFds: 31 })
  assert.deepEqual(parseProcessMetrics(''), { cpuSeconds: null, residentBytes: null, startTime: null, threads: null, openFds: null })
})

test('the coverage note names the span of the retained records and their count', () => {
  assert.equal(coverageText('2026-10-23T08:00:00Z', '2026-10-23T11:00:00Z', 1234), '最近 3.0 小时 · 1,234 条')
  assert.equal(coverageText('2026-10-23T10:59:30Z', '2026-10-23T11:00:00Z', 2), '最近 1 分钟 · 2 条')
  assert.equal(coverageText('2026-10-20T11:00:00Z', '2026-10-23T11:00:00Z', 9), '最近 3.0 天 · 9 条')
  assert.equal(coverageText(undefined, '2026-10-23T11:00:00Z', 0), '审计保留记录 · 0 条')
})

test('the shell keeps the localStorage keys under the rtdns- prefix and shows no MosDNS name', () => {
  const rules = ['components/rules', 'pages/rules'].flatMap((dir) => readdirSync(new URL(`../src/${dir}`, import.meta.url)).map((name) => `../src/${dir}/${name}`))
  const sources = ['../src/App.vue', '../src/utils/theme.js', '../src/utils/autoRefresh.js', '../src/pages/system/SystemBackupSection.vue', ...rules]
  for (const path of sources) {
    const source = read(path)
    for (const match of source.matchAll(/(?:STORAGE_KEY|_KEY) = '([^']+)'/g)) assert.match(match[1], /^rtdns-/, `${path}: ${match[1]}`)
    assert.doesNotMatch(source, /MosDNS/, path)
  }
  assert.match(read('../index.html'), /<title>rtdns<\/title>/)
})

test('behaviour-sensitive table CSS is not reintroduced by the shell', () => {
  const css = read('../src/style.css')
  const shell = css.slice(css.indexOf('/* ---- rtdns shell'))
  assert.ok(shell.length > 0)
  assert.doesNotMatch(shell, /table-layout:\s*fixed|overflow-wrap:\s*anywhere|calc\([^)]*%/)
})

test('the diagnostic capture is its own page, reached from and returning to the live query page', () => {
  const live = read('../src/pages/LogsPage.vue')
  const diagnostic = read('../src/pages/LogsDiagnosticPage.vue')
  assert.match(live, /mode="live"/)
  assert.doesNotMatch(live, /mode="diagnostic"/, 'the live page never shows the diagnostic panel')
  assert.match(live, /router\.push\('\/logs\/diagnostic'\)/)
  assert.match(diagnostic, /mode="diagnostic"/)
  assert.doesNotMatch(diagnostic, /mode="live"/, 'the diagnostic page never shows the live query')
  assert.match(diagnostic, /router\.push\('\/logs'\)/)
  assert.match(read('../src/components/QueryManager.vue'), /返回实时查询/)
  // The sidebar keeps 日志 highlighted below /logs.
  assert.match(read('../src/App.vue'), /route\.path\.startsWith\(`\/\$\{module\.id\}\/`\)/)
})

test('the system page stacks its five sections in setting rows', () => {
  const page = read('../src/pages/system/SystemPage.vue')
  const order = ['SystemUpdateSection', 'SystemBackupSection', 'SystemAuditSection', 'SystemUiSection', 'SystemServiceSection']
  const positions = order.map((name) => page.indexOf(`<${name} />`))
  assert.ok(positions.every((position) => position > 0), 'every section is mounted')
  assert.deepEqual([...positions].sort((a, b) => a - b), positions, 'in the decided order')
  const titles = { Update: '版本与更新', Backup: '配置与备份', Audit: '审计日志', Ui: '界面', Service: '服务' }
  for (const [name, title] of Object.entries(titles)) {
    const section = read(`../src/pages/system/System${name}Section.vue`)
    assert.match(section, new RegExp(`<SettingSection id="[a-z]+" title="${title}">`), name)
    assert.match(section, /<SettingRow /, name)
  }
})

test('the rule pages show no internal terms or Go-era names', () => {
  for (const dir of ['components/rules', 'pages/rules']) {
    for (const name of readdirSync(new URL(`../src/${dir}`, import.meta.url))) {
      const source = read(`../src/${dir}/${name}`)
      // Visible text only: text nodes and the text attributes of the template part.
      const template = source.slice(source.indexOf('<template>'))
      const visible = [...template.matchAll(/>([^<>]+)</g), ...template.matchAll(/\b(?:hint|description|label|title|placeholder|aria-label)="([^"]+)"/g)].map((match) => match[1].replace(/\{\{.*?\}\}/g, '')).join('\n')
      assert.doesNotMatch(visible, /requery|route_learner|domain_output|switch\d|MosDNS|Go 版|legacy|兼容分支|unified_matcher/, `${dir}/${name}`)
    }
  }
})

test('user-facing texts avoid internal terms', () => {
  for (const path of ['../src/pages/system/SystemAuditSection.vue', '../src/pages/OverviewPage.vue', '../src/pages/system/SystemUpdateSection.vue']) {
    assert.doesNotMatch(read(path), /retained ring|v2 统计|时间窗|热数据/, path)
  }
  assert.match(read('../src/pages/system/SystemAuditSection.vue'), /修改上限会清空已保存的详细日志/)
  assert.doesNotMatch(read('../src/components/QueryManager.vue'), /192\.168\./)
})
