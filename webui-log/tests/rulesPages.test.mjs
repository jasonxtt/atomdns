import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { createMemoryHistory } from 'vue-router'

import { changeFailure, changeMessage, flagsForPage, optionLabel, pageOfFlag, runFlagChange } from '../src/api/flags.js'
import { RULE_SECTIONS } from '../src/navigation.js'
import { createAppRouter } from '../src/router.js'
import { aliasMapFromRows, aliasRowsFrom, aliasesChanged, normalizeAliasMap } from '../src/utils/clientAliases.js'
import { LIST_LINE_CAP, canSaveList, canonicalRules, filterLines, readList, sameRules, showUrl } from '../src/utils/ruleList.js'
import { ROUTING_SUBSCRIPTIONS, SUBSCRIPTION_FORMATS, inferFromUrl, isDownloaded, ruleFields, sortRules, withFormat } from '../src/utils/subscription.js'
import { clearDirty, hasUnsaved, resetUnsaved, setDirty } from '../src/utils/unsaved.js'

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')

// ----- flags: which page shows which flag ---------------------------------------------

const flag = (key, section, extra = {}) => ({ key, label: key, section, kind: 'bool', value: false, default: false, ...extra })

test('a flag is shown on the sub-item of its declared section, in declaration order', () => {
  const flags = [
    flag('core_mode', 'mode'), flag('dns_mode', 'mode'), flag('ddns_foreign', 'upstream'), flag('block_empty', 'blocking'),
    flag('block_ads', 'blocking'), flag('lazy_cache', 'cache'), flag('client_routing', 'client'), flag('learn_extra', 'learning'),
    flag('redirect_extra', 'rewrite'), flag('route_extra', 'routing'), flag('group_extra', 'special_groups'), flag('prefer_v4', 'mode')
  ]
  assert.deepEqual(flagsForPage(flags, 'mode').map((item) => item.key), ['core_mode', 'dns_mode', 'prefer_v4'])
  assert.deepEqual(flagsForPage(flags, 'blocking').map((item) => item.key), ['block_empty', 'block_ads'])
  assert.deepEqual(flagsForPage(flags, 'clients').map((item) => item.key), ['client_routing'])
  assert.deepEqual(flagsForPage(flags, 'special-groups').map((item) => item.key), ['group_extra'])
  assert.deepEqual(flagsForPage(flags, 'learning').map((item) => item.key), ['learn_extra'])
  // A rewrite flag belongs to 分流规则 now, after the routing flags that were declared before it.
  assert.deepEqual(flagsForPage(flags, 'routing').map((item) => item.key), ['redirect_extra', 'route_extra'])
  assert.equal(pageOfFlag(flag('x', 'nowhere')), '')
  assert.deepEqual(flagsForPage(undefined, 'mode'), [])
})

test('every page a flag can land on is a sub-item of the sidebar', () => {
  const ids = new Set(RULE_SECTIONS.map((item) => item.id))
  for (const section of ['mode', 'upstream', 'routing', 'rewrite', 'blocking', 'learning', 'cache', 'client', 'special_groups']) {
    assert.ok(ids.has(pageOfFlag(flag('k', section))), section)
  }
})

test('a change tells whether the cache refresh was started; failures explain themselves', async () => {
  const core = flag('core_mode', 'mode', { label: '核心运行模式' })
  assert.match(changeMessage(core, { requery_triggered: true }), /正在刷新缓存与预热/)
  assert.doesNotMatch(changeMessage(core, { requery_triggered: false }), /预热/)
  assert.doesNotMatch(changeMessage(core, {}), /预热/)
  // 409: another change was being applied; the page reads again and asks for a retry.
  const conflict = changeFailure(core, Object.assign(new Error('x'), { status: 409 }))
  assert.equal(conflict.reload, true)
  assert.match(conflict.message, /重新读取，请再点一次/)
  // 503: an online update is being tried out; the backend's reason is shown as it is.
  const trial = changeFailure(core, Object.assign(new Error('配置更新试运行中，暂不能修改开关'), { status: 503 }))
  assert.equal(trial.message, '配置更新试运行中，暂不能修改开关')
  assert.equal(trial.reload, false)
  const other = changeFailure(core, Object.assign(new Error('boom'), { status: 500 }))
  assert.match(other.message, /boom/)

  assert.deepEqual(await runFlagChange(core, async () => ({ requery_triggered: true })), { ok: true, message: changeMessage(core, { requery_triggered: true }), reload: true })
  const failed = await runFlagChange(core, async () => { throw Object.assign(new Error('x'), { status: 409 }) })
  assert.equal(failed.ok, false)
  assert.equal(failed.reload, true)
  assert.equal(optionLabel({ options: [{ value: 'a', label: 'A 档' }] }, 'a'), 'A 档')
  assert.equal(optionLabel({ options: [] }, 'zz'), 'zz')
})

test('the switches wait for each other while one change is being applied, and restore uses DELETE', () => {
  const source = read('../src/components/rules/FlagSwitches.vue')
  assert.match(source, /if \(busyKey\.value\) return/)
  assert.match(source, /busyKey\.value = flag\.key/)
  // Every control of the page is disabled meanwhile: the toggle, each segment and the restore button.
  assert.equal(source.match(/:disabled="Boolean\(busyKey\)"/g).length, 3)
  assert.match(source, /应用中…/)
  assert.match(source, /deleteRequest\(`\/api\/v1\/flags\//)
  assert.match(source, /postJSON\(`\/api\/v1\/flags\/\$\{encodeURIComponent\(flag\.key\)\}`, \{ value \}\)/)
  assert.match(source, /已修改/)
  assert.match(source, /恢复默认/)
  // The old front-end follow-ups are gone: the host starts the refresh.
  assert.doesNotMatch(source, /requery\/trigger|cache_all|flush/)
})

// ----- the shared list editor ----------------------------------------------------------

test('a list longer than the cap is cut and can never be saved', () => {
  const full = Array.from({ length: LIST_LINE_CAP }, (_, index) => `d${index}.example`).join('\n')
  const fits = readList(`${full}\n`)
  assert.equal(fits.truncated, false)
  assert.equal(fits.count, LIST_LINE_CAP)
  const over = readList(`${full}\nlast.example\n`)
  assert.equal(over.truncated, true)
  assert.equal(over.text.split('\n').length, LIST_LINE_CAP)
  assert.equal(canSaveList({ truncated: true, dirty: true }), false, 'a cut list is read-only')
  assert.equal(canSaveList({ truncated: false, dirty: false }), false, 'nothing to save')
  assert.equal(canSaveList({ truncated: false, dirty: true }), true)
  assert.equal(canSaveList({ truncated: false, dirty: true, saving: true }), false)
  assert.equal(canSaveList({ truncated: false, dirty: true, loading: true }), false)
  assert.equal(canSaveList({ truncated: false, dirty: false, uncertain: true }), true, 'an unconfirmed list can be confirmed')
  // `show` is asked for one line more than the cap, which is how a cut list is noticed.
  assert.equal(showUrl('whitelist'), `/plugins/whitelist/show?limit=${LIST_LINE_CAP + 1}`)
  assert.equal(showUrl('a b'), `/plugins/a%20b/show?limit=${LIST_LINE_CAP + 1}`)
})

test('blank lines and spacing are not changes; search only filters what is shown', () => {
  assert.equal(canonicalRules('  a.example \n\n b.example\n'), 'a.example\nb.example')
  assert.equal(sameRules('a.example\nb.example\n', ' a.example\n\nb.example'), true)
  assert.equal(sameRules('a.example', 'a.example\nb.example'), false)
  const text = 'full:A.example\ndomain:b.example\nkeyword:abc'
  assert.deepEqual(filterLines(text, 'EXAMPLE'), ['full:A.example', 'domain:b.example'])
  assert.deepEqual(filterLines(text, ''), [], 'an empty search shows the whole text instead')
  assert.equal(text, 'full:A.example\ndomain:b.example\nkeyword:abc', 'the content is untouched')
})

test('the editor posts the rules, shows the server error as it is, reads back and keeps drafts when unsure', () => {
  const source = read('../src/components/rules/ListEditor.vue')
  assert.match(source, /postJSON\(`\/plugins\/\$\{encodeURIComponent\(props\.tag\)\}\/post`, \{ values \}\)/)
  assert.match(source, /error\.value = `保存失败: \$\{failure\.message\}`/, 'the backend error text is shown unchanged')
  assert.match(source, /部分规则未被服务器接受/)
  assert.match(source, /uncertain\.value = true/)
  assert.match(source, /data-list-truncated/)
  // The textarea is read-only while the list is cut, and an auto refresh never replaces an edit.
  assert.match(source, /:readonly="!canWrite \|\| !loaded \|\| truncated"/)
  assert.match(source, /:disabled="loading \|\| saving"/, 'no typing while the save is in flight')
  // Nothing is saved before the list was read completely once (a save replaces the whole list).
  assert.match(source, /canWrite\.value && loaded\.value && !missing\.value/)
  assert.match(source, /loaded\.value = true/)
  assert.match(source, /if \(dirty\.value \|\| uncertain\.value \|\| saving\.value\) return/)
  assert.match(source, /setDirty\(uid, dirty\.value \|\| uncertain\.value\)/)
  assert.match(source, /clearDirty\(uid\)/)
})

// ----- unsaved changes ----------------------------------------------------------------

test('unsaved changes are tracked per editor', () => {
  resetUnsaved()
  assert.equal(hasUnsaved(), false)
  setDirty('a', true)
  setDirty('b', true)
  clearDirty('a')
  assert.equal(hasUnsaved(), true)
  setDirty('b', false)
  assert.equal(hasUnsaved(), false)
})

async function guardedRouter(answer) {
  const asked = []
  const router = createAppRouter({
    history: createMemoryHistory(),
    pageComponent: (name) => ({ name, render: () => null }),
    confirmLeave: async () => { asked.push(true); return answer }
  })
  await router.push('/rules/routing')
  return { router, asked }
}

test('leaving a page with unsaved changes asks first and stays when declined', async () => {
  resetUnsaved()
  const declined = await guardedRouter(false)
  setDirty('list', true)
  await declined.router.push('/rules/blocking')
  assert.equal(declined.router.currentRoute.value.path, '/rules/routing', 'declined: stay')
  assert.equal(declined.asked.length, 1)
  resetUnsaved()
  const accepted = await guardedRouter(true)
  setDirty('list', true)
  await accepted.router.push('/rules/blocking')
  assert.equal(accepted.router.currentRoute.value.path, '/rules/blocking', 'accepted: leave')
  assert.equal(accepted.asked.length, 1)
  resetUnsaved()
})

test('nothing is asked without unsaved changes; opening another special group with edits asks', async () => {
  resetUnsaved()
  const { router, asked } = await guardedRouter(false)
  await router.push('/rules/blocking')
  assert.equal(asked.length, 0)
  await router.push({ path: '/rules/special-groups', query: { group: '50' } })
  assert.equal(asked.length, 0, 'no edits, no question')
  setDirty('group', true)
  await router.push({ path: '/rules/special-groups', query: { group: '51' } })
  assert.equal(asked.length, 1, 'another group of the same page asks too')
  assert.equal(router.currentRoute.value.query.group, '50', 'declined: stay on the group')
  await router.push({ path: '/rules/special-groups', query: { group: '50' } })
  assert.equal(asked.length, 1, 'the same address is no move')
  resetUnsaved()
})

// ----- subscriptions and aliases -------------------------------------------------------

test('the routing page lists the five online sets', () => {
  assert.deepEqual(ROUTING_SUBSCRIPTIONS.map((item) => item.tag), ['geosite_cn', 'geosite_no_cn', 'geoip_cn', 'cuscn', 'cusnocn'])
  assert.equal(new Set(ROUTING_SUBSCRIPTIONS.map((item) => item.type)).size, 5)
})

test('a new subscription takes its name and file from the address', () => {
  assert.deepEqual(inferFromUrl('https://example.com/rules/geosite-cn.srs?x=1'), { name: 'geosite-cn', files: 'srs/geosite-cn.srs' })
  assert.equal(inferFromUrl(''), null)
  assert.equal(inferFromUrl('https://example.com/'), null)
})

test('only an .srs address keeps its file name; other formats are stored as converted text', () => {
  assert.deepEqual(inferFromUrl('https://example.com/a/ads.yaml'), { name: 'ads', files: 'srs/ads.txt' })
  assert.deepEqual(inferFromUrl('https://example.com/a/list.json?x=1'), { name: 'list', files: 'srs/list.txt' })
  assert.deepEqual(inferFromUrl('https://example.com/hosts.txt'), { name: 'hosts', files: 'srs/hosts.txt' })
  assert.deepEqual(inferFromUrl('https://example.com/rules'), { name: 'rules', files: 'srs/rules.txt' })
  assert.deepEqual(inferFromUrl('https://example.com/GEO.SRS'), { name: 'GEO', files: 'srs/GEO.SRS' })
})

test('the format of a subscription is saved only when it is not auto', () => {
  assert.deepEqual(SUBSCRIPTION_FORMATS.map((item) => item.value), ['auto', 'srs', 'text', 'yaml', 'json'])
  const payload = { name: 'a', url: 'https://example.com/a.yaml' }
  assert.deepEqual(withFormat({}, payload, 'auto'), payload, 'auto sends no field')
  assert.deepEqual(withFormat({}, payload, undefined), payload)
  assert.deepEqual(withFormat({}, payload, 'yaml'), { ...payload, format: 'yaml' })
  assert.deepEqual(withFormat({ name: 'old', format: 'json', rule_count: 3 }, payload, 'text'), { ...payload, rule_count: 3, format: 'text' })
  assert.equal('format' in withFormat({ format: 'json' }, payload, 'auto'), false, 'switching back to auto clears the stored format')
  const source = readFileSync(new URL('../src/components/rules/SubscriptionTable.vue', import.meta.url), 'utf8')
  assert.match(source, /v-model="editor\.format"/)
  assert.match(source, /format: String\(rule\.format \|\| 'auto'\)/, 'the stored format is shown when editing')
})

test('a subscription counts as downloaded with a rule count or an update time', () => {
  assert.equal(isDownloaded({ rule_count: 0, last_updated: '0001-01-01T00:00:00Z' }), false)
  assert.equal(isDownloaded({ rule_count: 3 }), true)
  assert.equal(isDownloaded({ rule_count: 0, last_updated: '2026-10-24T01:02:03Z' }), true)
  assert.equal(isDownloaded(undefined), false)
  assert.deepEqual(ruleFields({ name: 'a', __tag: 'x', __label: 'y' }), { name: 'a' }, 'page-only fields are not saved')
  assert.deepEqual(sortRules([{ name: 'b', enabled: false }, { name: 'a', enabled: true }]).map((rule) => rule.name), ['a', 'b'])
})

test('client aliases: rows list saved aliases and recent clients; empty names are dropped', () => {
  const map = normalizeAliasMap({ '::ffff:192.0.2.10': ' Phone ', '192.0.2.11': '', 'bad': null })
  assert.deepEqual(map, { '192.0.2.10': 'Phone' })
  const rows = aliasRowsFrom(map, ['192.0.2.9', '::ffff:192.0.2.10', '', null])
  assert.deepEqual(rows.map((row) => row.ip), ['192.0.2.9', '192.0.2.10'])
  assert.equal(aliasesChanged(rows), false)
  rows[0].alias = 'Tablet'
  rows[1].alias = ''
  assert.equal(aliasesChanged(rows), true)
  assert.deepEqual(aliasMapFromRows(rows), { '192.0.2.9': 'Tablet' })
})

test('whole-value saves wait until the current value was read once', () => {
  // A save of the aliases, the overrides or the timer replaces the whole stored value.
  const aliases = read('../src/components/rules/ClientAliasPanel.vue')
  assert.match(aliases, /getJSON\('\/plugins\/clientname'\),/, 'a failed alias read is an error, not an empty map')
  assert.match(aliases, /if \(!loaded\.value\) throw new Error/)
  assert.match(aliases, /:disabled="loading \|\| saving \|\| !loaded \|\| !dirty"/)
  const overrides = read('../src/components/rules/OverridesPanel.vue')
  assert.match(overrides, /!supportsOperation\('system\.global_overrides'\) \|\| !loaded\.value/)
  assert.match(overrides, /:disabled="applying \|\| !loaded" @click="saveOverrides"/)
  assert.match(read('../src/components/rules/RequeryPanel.vue'), /if \(!config\.value\) return setError/)
})

test('the log page sends alias editing to the clients sub-item', () => {
  const logs = read('../src/components/QueryManager.vue')
  assert.match(logs, /to="\/rules\/clients"/)
  assert.doesNotMatch(logs, /putJSON|openAliasManager|aliasModalOpen/)
})

// ----- the pages -----------------------------------------------------------------------

test('each sub-item page has its switches first, then lists and subscriptions', () => {
  const expected = {
    Mode: ['FlagSwitches page="mode"', 'tag="realiplist"', 'tag="cnfakeipfilter"'],
    Upstream: ['FlagSwitches page="upstream"', '<UpstreamPanel />', '<OverridesPanel />'],
    Routing: ['FlagSwitches page="routing"', 'tag="whitelist"', 'tag="greylist"', 'tag="direct_ip"', 'tag="rewrite"', 'tag="ddnslist"', 'kind="online"', '<SpecialGroupLinks />'],
    Blocking: ['FlagSwitches page="blocking"', 'tag="blocklist"', 'kind="adguard"'],
    Learning: ['FlagSwitches page="learning"', '<DomainGenerationPanel />', '<LearningData />', '<RequeryPanel />'],
    Cache: ['FlagSwitches page="cache"', '<CachePanel />', 'to="/rules/learning"'],
    Clients: ['FlagSwitches page="clients"', 'tag="client_ip"', '<ClientAliasPanel />'],
    SpecialGroups: ['FlagSwitches page="special-groups"', '<SpecialGroupDetail']
  }
  for (const [name, parts] of Object.entries(expected)) {
    const source = read(`../src/pages/rules/Rules${name}Page.vue`)
    let at = -1
    for (const part of parts) {
      const next = source.indexOf(part, at + 1)
      assert.ok(next > at, `${name}: ${part} (in order)`)
      at = next
    }
    assert.doesNotMatch(source, /RulesManager|ListManager|UpstreamManager|DataManagementManager/, name)
  }
})

test('the special group page manages everything of a group in one place', () => {
  const detail = read('../src/components/rules/SpecialGroupDetail.vue')
  assert.match(detail, /ListEditor :tag="group\.manual_plugin_tag"/)
  assert.match(detail, /<SubscriptionTable kind="online"/)
  assert.match(detail, /diversion_plugin_tag/)
  assert.match(detail, /deleteRequest\(`\/api\/v1\/special-groups\/\$\{props\.group\.slot\}`\)/)
  assert.match(detail, /它的上游、手动名单、订阅和监听端口都会一起删除/, 'the delete confirmation names what goes with the group')
  assert.match(detail, /upstream_sources/)
  assert.match(detail, /owned/)
  assert.match(detail, /port === 53/)
  // The group's own upstreams go through the same read-only check as the ordinary ones, and a
  // refreshed group never replaces a form that has unsaved edits.
  assert.match(detail, /isNativeUpstreamReadOnly\(item \|\| \{\}, props\.capabilities\)/)
  assert.match(detail, /ownedStatus\(item\)\.readOnly/)
  assert.match(detail, /if \(dirty\.value \|\| !next\) return/)
  // The baseline after a save is what the request submitted, so edits typed while it was in flight stay unsaved.
  assert.match(detail, /const submitted = JSON\.stringify\(form\)/)
  assert.match(detail, /baseline\.value = submitted/)
  assert.doesNotMatch(detail, /baseline\.value = JSON\.stringify\(form\)/)
  const page = read('../src/pages/rules/RulesSpecialGroupsPage.vue')
  assert.match(page, /:key="selected"/, 'saving a group does not remount its lists')
})

test('the learning page offers the entries, whole-category clearing and no single-entry delete', () => {
  const data = read('../src/components/rules/LearningData.vue')
  assert.match(data, /\/plugins\/route_learner\/entries\?/)
  assert.match(data, /\/plugins\/\$\{encodeURIComponent\(item\.tag\)\}\/flush/)
  assert.match(data, /openConfirm\(`确定清空/)
  assert.doesNotMatch(data, /deleteRequest|method: 'DELETE'/)
})

test('the old rule-page components are gone', () => {
  for (const name of ['ListManager', 'RulesManager', 'UpstreamManager', 'DataManagementManager']) {
    assert.throws(() => read(`../src/components/${name}.vue`), /ENOENT/, name)
  }
  assert.doesNotMatch(read('../src/utils/nativeSettings.js'), /NATIVE_REPLACEMENTS_NOTE/)
})

// ----- upstream groups that are not in use (show_when) -------------------------------

test('the groups answer becomes a map and degrades to "all in use"', async () => {
  const { loadUpstreamGroups } = await import('../src/utils/upstreamStats.js')
  const caps = { endpoints: { upstream: { groups_get: true } } }
  const answer = [{ tag: 'cnfake', active: false, inactive_reason: '仅在「国内域名 FakeIP」开启时使用', used_when: '「国内域名 FakeIP」开启' }, { tag: 'domestic', active: true }, { nope: 1 }]
  assert.deepEqual(await loadUpstreamGroups(async () => answer, caps), {
    cnfake: { active: false, inactive_reason: '仅在「国内域名 FakeIP」开启时使用', used_when: '「国内域名 FakeIP」开启' },
    domestic: { active: true, inactive_reason: '', used_when: '' }
  })
  assert.deepEqual(await loadUpstreamGroups(async () => answer, { endpoints: { upstream: {} } }), {}, 'an older host')
  assert.deepEqual(await loadUpstreamGroups(async () => { throw new Error('x') }, caps), {})
  assert.deepEqual(await loadUpstreamGroups(async () => ({}), caps), {})
})

test('a change that leaves a group without an enabled upstream is refused with the reason', async () => {
  const { lastUpstreamReason } = await import('../src/utils/upstreamStats.js')
  const on = { tag: 'a', enabled: true }
  const off = { tag: 'b', enabled: false }
  assert.equal(lastUpstreamReason('cnfake', [on, off]), '')
  assert.equal(lastUpstreamReason('cnfake', [off], null, '国内 FakeIP 上游'), '上游组「国内 FakeIP 上游」至少要保留一个启用的上游。')
  assert.equal(
    lastUpstreamReason('cnfake', [], { active: false, inactive_reason: '仅在「国内域名 FakeIP」开启时使用' }, '国内 FakeIP 上游'),
    '上游组「国内 FakeIP 上游」至少要保留一个启用的上游。这个组只在「国内域名 FakeIP」开启时使用；不想用它，关闭该开关即可。'
  )
  // A group in use (switch on) still names its switch.
  assert.equal(
    lastUpstreamReason('cnfake', [off], { active: true, inactive_reason: '', used_when: '「国内域名 FakeIP」开启' }, '国内 FakeIP 上游'),
    '上游组「国内 FakeIP 上游」至少要保留一个启用的上游。这个组只在「国内域名 FakeIP」开启时使用；不想用它，关闭该开关即可。'
  )
})

test('the upstream page hides groups not in use unless asked, and checks before disabling or deleting the last one', () => {
  const panel = read('../src/components/rules/UpstreamPanel.vue')
  assert.match(panel, /loadUpstreamGroups\(getJSON, capabilities\.value\)/)
  assert.match(panel, /configuredGroups\.value\.filter\(\(group\) => showInactive\.value \|\| !inactive\(group\)\)/)
  assert.match(panel, /显示未生效的组（\$\{inactiveCount\}）/)
  assert.match(panel, /data-inactive-group/)
  assert.match(panel, /rtdns-show-inactive-upstream-groups/)
  // toggle and remove compute the list first and refuse before any request.
  for (const name of ['toggle', 'remove']) {
    const body = panel.slice(panel.indexOf(`async function ${name}(`))
    const refusal = body.indexOf('refusal(group, list)')
    assert.ok(refusal > 0 && refusal < body.indexOf('saveGroup(group, list)'), name)
  }
  // A refused click puts the switch back (the data did not change, so Vue would not).
  assert.match(panel, /@change="toggle\(group, row, \$event\)"/)
  const toggleBody = panel.slice(panel.indexOf('async function toggle('), panel.indexOf('async function remove('))
  assert.equal((toggleBody.match(/resetSwitch\(input, group, row\.index\)/g) || []).length, 3)
  const overview = read('../src/pages/OverviewPage.vue')
  assert.match(overview, /filter\(\(group\) => upstreamGroupStates\.value\[group\]\?\.active !== false\)/)
  const upstream = read('../src/pages/rules/RulesUpstreamPage.vue')
  const routing = read('../src/pages/rules/RulesRoutingPage.vue')
  assert.doesNotMatch(upstream, /tag="ddnslist"/)
  assert.match(routing, /运行模式 › DDNS 优先国外解析/)
})
