<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { getJSON, getText } from '../api/http'
import { pageOfFlag } from '../api/flags'
import { supportsOperation } from '../api/runtimeCapabilities'
import { NAV_MODULES, PAGES, inModule } from '../navigation'
import { LIST_LINE_CAP, showUrl } from '../utils/ruleList'
import { normalizeAliasMap } from '../utils/clientAliases'
import { SEARCH_KINDS, SEARCH_LISTS, aliasEntries, flagEntries, groupEntries, listEntries, logEntry, looksLikeLookup, normalizeQuery, pick } from '../utils/globalSearch'
import AppIcon from './AppIcon.vue'

// Ctrl/Cmd+K opens the search palette: pages, settings, client names and addresses, domain and
// address lists, special groups and the query log. The small sources (settings, aliases, groups)
// are read when the palette opens, the lists when the first word is typed, and the log is asked
// for each word (the server filters it).
const router = useRouter()
const open = ref(false)
const query = ref('')
const active = ref(0)
const input = ref(null)
const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || '')

const pageEntries = PAGES.map((page) => {
  const module = NAV_MODULES.find((item) => inModule(item.id, page.path))
  const group = module && module.label !== page.title ? module.label : ''
  return { id: `page:${page.path}`, kind: 'page', title: page.title, detail: group, keys: [group + page.title, page.title], icon: module?.items.find((item) => item.path === page.path)?.icon || module?.icon || 'list', route: page.path }
})
const KIND_ICON = { flag: 'sliders', client: 'users', domain: 'list', iplist: 'list', group: 'layers', log: 'search' }

const flags = ref([])
const aliases = ref({})
const groups = ref([])
const listTexts = ref({})
let listsLoadedAt = 0
let listsKey = ''
let smallReady = Promise.resolve()
const logHits = ref([])
const loading = ref(false)
let lookupSeq = 0
let logTimer = 0

async function loadSmall() {
  const [flagList, aliasMap, groupList] = await Promise.allSettled([
    supportsOperation('flags.manage') ? getJSON('/api/v1/flags') : Promise.reject(new Error('unavailable')),
    supportsOperation('client.aliases') ? getJSON('/plugins/clientname') : Promise.reject(new Error('unavailable')),
    supportsOperation('groups.read') ? getJSON('/api/v1/special-groups') : Promise.reject(new Error('unavailable'))
  ])
  flags.value = flagList.status === 'fulfilled' && Array.isArray(flagList.value?.flags) ? flagList.value.flags : []
  aliases.value = aliasMap.status === 'fulfilled' ? normalizeAliasMap(aliasMap.value) : {}
  groups.value = groupList.status === 'fulfilled' && Array.isArray(groupList.value) ? groupList.value : []
}

// Every manual list the pages edit, including the lists of the special groups.
function allLists() {
  const extra = groups.value
    .filter((group) => group.manual_plugin_tag)
    .map((group) => ({ tag: group.manual_plugin_tag, title: `${group.name}的手动名单`, group: String(group.slot) }))
  return [...SEARCH_LISTS, ...extra]
}

async function loadLists() {
  if (!supportsOperation('rules.local.read')) return
  // The groups tell which lists exist, so they are read first.
  await smallReady
  const lists = allLists()
  const key = lists.map((list) => list.tag).join(',')
  if (key === listsKey && Date.now() - listsLoadedAt < 30000) return
  listsKey = key
  listsLoadedAt = Date.now()
  const texts = await Promise.allSettled(lists.map((list) => getText(showUrl(list.tag, LIST_LINE_CAP))))
  const next = {}
  lists.forEach((list, index) => { if (texts[index].status === 'fulfilled') next[list.tag] = texts[index].value })
  listTexts.value = next
}

function scheduleLookup() {
  clearTimeout(logTimer)
  const needle = normalizeQuery(query.value)
  const seq = ++lookupSeq
  if (!looksLikeLookup(needle) || !supportsOperation('audit.read')) {
    logHits.value = []
    loading.value = false
    return
  }
  loading.value = true
  logTimer = setTimeout(async () => {
    try {
      const params = new URLSearchParams({ page: '1', limit: '5', q: needle })
      const data = await getJSON(`/api/v2/audit/logs?${params.toString()}`)
      if (seq === lookupSeq) logHits.value = (Array.isArray(data?.logs) ? data.logs : []).map((log) => logEntry(log, aliases.value))
    } catch {
      if (seq === lookupSeq) logHits.value = []
    } finally {
      if (seq === lookupSeq) loading.value = false
    }
  }, 250)
}

function listRoute(list, line) {
  if (list.group) return { path: '/rules/special-groups', query: { group: list.group, list: list.tag, find: line } }
  return { path: `/rules/${list.page}`, query: { ...(list.tab ? { tab: list.tab } : {}), list: list.tag, find: line } }
}

const sections = computed(() => {
  const needle = normalizeQuery(query.value)
  if (!needle) return [{ kind: 'page', label: '页面', items: pageEntries }]
  const found = {
    page: pick(pageEntries, needle),
    flag: pick(flagEntries(flags.value, pageOfFlag), needle),
    client: pick(aliasEntries(aliases.value), needle),
    group: pick(groupEntries(groups.value), needle),
    domain: [],
    iplist: [],
    log: logHits.value
  }
  for (const list of allLists()) {
    if (listTexts.value[list.tag] === undefined) continue
    const target = list.ip ? found.iplist : found.domain
    for (const entry of listEntries(list, listTexts.value[list.tag], needle, listRoute)) if (target.length < 12) target.push(entry)
  }
  found.domain = found.domain.slice(0, 8)
  found.iplist = found.iplist.slice(0, 8)
  return SEARCH_KINDS.map(({ kind, label }) => ({ kind, label, items: found[kind] })).filter((section) => section.items.length > 0)
})
const flat = computed(() => sections.value.flatMap((section) => section.items))
const indexOf = (entry) => flat.value.indexOf(entry)
watch(flat, () => { active.value = 0 })
watch(query, () => {
  scheduleLookup()
  if (normalizeQuery(query.value)) loadLists()
})

async function show() {
  open.value = true
  query.value = ''
  active.value = 0
  logHits.value = []
  await nextTick()
  input.value?.focus()
  smallReady = loadSmall()
}
function hide() {
  open.value = false
  clearTimeout(logTimer)
  lookupSeq += 1
}
function go(entry) {
  if (!entry) return
  hide()
  router.push(entry.route)
}
function onInputKey(event) {
  if (event.key === 'ArrowDown') { active.value = Math.min(flat.value.length - 1, active.value + 1); event.preventDefault() }
  else if (event.key === 'ArrowUp') { active.value = Math.max(0, active.value - 1); event.preventDefault() }
  else if (event.key === 'Enter') { go(flat.value[active.value]); event.preventDefault() }
}
function onGlobalKey(event) {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
    event.preventDefault()
    open.value ? hide() : show()
  } else if (event.key === 'Escape' && open.value) {
    hide()
  }
}
onMounted(() => window.addEventListener('keydown', onGlobalKey))
onBeforeUnmount(() => {
  window.removeEventListener('keydown', onGlobalKey)
  clearTimeout(logTimer)
})
</script>

<template>
  <button type="button" class="rt-search" aria-label="全局搜索" @click="show">
    <AppIcon name="search" :size="18" />
    <span class="rt-search-text">搜索页面、设置、域名、客户端…</span>
    <kbd class="rt-kbd">{{ isMac ? '⌘K' : 'Ctrl K' }}</kbd>
  </button>

  <div v-if="open" class="rt-palette-backdrop" @mousedown.self="hide">
    <div class="rt-palette" role="dialog" aria-modal="true" aria-label="全局搜索">
      <div class="rt-palette-input">
        <AppIcon name="search" :size="18" />
        <input ref="input" v-model="query" type="text" placeholder="搜索页面、设置、域名、客户端 IP…" autocomplete="off" aria-label="全局搜索" @keydown="onInputKey" />
      </div>
      <ul class="rt-palette-list" role="listbox">
        <template v-for="section in sections" :key="section.kind">
          <li class="rt-palette-heading" role="presentation">{{ section.label }}</li>
          <li
            v-for="entry in section.items"
            :key="entry.id"
            role="option"
            :aria-selected="indexOf(entry) === active"
            :class="{ active: indexOf(entry) === active }"
            @mousemove="active = indexOf(entry)"
            @click="go(entry)"
          >
            <AppIcon :name="entry.icon || KIND_ICON[entry.kind] || 'list'" :size="18" />
            <span class="rt-palette-title" :class="{ mono: entry.kind === 'domain' || entry.kind === 'iplist' || entry.kind === 'log' }">{{ entry.title }}</span>
            <span v-if="entry.detail" class="rt-palette-group">{{ entry.detail }}</span>
          </li>
        </template>
        <li v-if="flat.length === 0" class="rt-palette-empty">{{ loading ? '搜索中…' : '没有匹配的结果' }}</li>
      </ul>
    </div>
  </div>
</template>
