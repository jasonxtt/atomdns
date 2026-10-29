<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { NAV_MODULES, PAGES, inModule } from '../navigation'
import AppIcon from './AppIcon.vue'

// Ctrl/Cmd+K opens a page-jump palette: every route, by its module and page name.
const router = useRouter()
const open = ref(false)
const query = ref('')
const active = ref(0)
const input = ref(null)
const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || '')

const entries = PAGES.map((page) => {
  const module = NAV_MODULES.find((item) => inModule(item.id, page.path))
  const group = module && module.label !== page.title ? module.label : ''
  return { path: page.path, title: page.title, group, icon: module?.items.find((item) => item.path === page.path)?.icon || module?.icon || 'list' }
})
const results = computed(() => {
  const needle = query.value.trim().toLowerCase()
  if (!needle) return entries
  return entries.filter((entry) => `${entry.group}${entry.title}`.toLowerCase().includes(needle))
})
watch(results, () => { active.value = 0 })

async function show() {
  open.value = true
  query.value = ''
  active.value = 0
  await nextTick()
  input.value?.focus()
}
function hide() { open.value = false }
function go(entry) {
  if (!entry) return
  hide()
  router.push(entry.path)
}
function onInputKey(event) {
  if (event.key === 'ArrowDown') { active.value = Math.min(results.value.length - 1, active.value + 1); event.preventDefault() }
  else if (event.key === 'ArrowUp') { active.value = Math.max(0, active.value - 1); event.preventDefault() }
  else if (event.key === 'Enter') { go(results.value[active.value]); event.preventDefault() }
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
onBeforeUnmount(() => window.removeEventListener('keydown', onGlobalKey))
</script>

<template>
  <button type="button" class="rt-search" aria-label="搜索页面" @click="show">
    <AppIcon name="search" :size="18" />
    <span class="rt-search-text">搜索页面…</span>
    <kbd class="rt-kbd">{{ isMac ? '⌘K' : 'Ctrl K' }}</kbd>
  </button>

  <div v-if="open" class="rt-palette-backdrop" @mousedown.self="hide">
    <div class="rt-palette" role="dialog" aria-modal="true" aria-label="搜索页面">
      <div class="rt-palette-input">
        <AppIcon name="search" :size="18" />
        <input ref="input" v-model="query" type="text" placeholder="搜索页面…" autocomplete="off" aria-label="搜索页面" @keydown="onInputKey" />
      </div>
      <ul class="rt-palette-list" role="listbox">
        <li
          v-for="(entry, index) in results"
          :key="entry.path"
          role="option"
          :aria-selected="index === active"
          :class="{ active: index === active }"
          @mousemove="active = index"
          @click="go(entry)"
        >
          <AppIcon :name="entry.icon" :size="18" />
          <span>{{ entry.title }}</span>
          <span v-if="entry.group" class="rt-palette-group">{{ entry.group }}</span>
        </li>
        <li v-if="results.length === 0" class="rt-palette-empty">没有匹配的页面</li>
      </ul>
    </div>
  </div>
</template>
