<script setup>
import { capabilityState, getRuntimeCapabilities, retryRuntimeCapabilities, supportsOperation } from './api/runtimeCapabilities'
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { getJSON } from './api/http'
import ConfirmBubbleHost from './components/ConfirmBubbleHost.vue'
import { NAV_MODULES, expandedAfterRoute, inModule } from './navigation'
import { loadAutoRefreshState } from './utils/autoRefresh'
import { installBeforeUnload } from './utils/unsaved'

const route = useRoute()
const router = useRouter()

// A module owns `/<id>` and everything below it (`#/logs/diagnostic` keeps 日志 highlighted).
const activeModule = computed(() => NAV_MODULES.find(module => inModule(module.id, route.path)) || NAV_MODULES[0])
// Sub-item lists (规则设置): collapsed by default, opened by clicking the module or by being on one of its pages.
const expanded = reactive(Object.fromEntries(NAV_MODULES.filter(module => module.items.length > 0).map(module => [module.id, false])))
const expandedModules = computed(() => NAV_MODULES.filter(module => module.items.length > 0 && expanded[module.id]))
watch(() => route.path, (path) => {
  for (const id of Object.keys(expanded)) expanded[id] = expandedAfterRoute(id, path, expanded[id])
}, { immediate: true })
function toggleModule(id) {
  expanded[id] = !expanded[id]
}
const pageTitle = computed(() => {
  const module = activeModule.value
  const title = route.meta?.title
  return title && title !== module.label ? `${module.label} · ${title}` : module.label
})
const versionText = computed(() => capabilityState.health?.version || '')

const topNotice = reactive({
  open: false,
  tone: 'success',
  message: ''
})

let autoRefreshTimerId = 0
let autoRefreshState = loadAutoRefreshState()
let topNoticeTimerId = 0
let removeBeforeUnload = () => {}

function stopAutoRefresh() {
  if (autoRefreshTimerId) {
    window.clearInterval(autoRefreshTimerId)
    autoRefreshTimerId = 0
  }
}

function startAutoRefresh() {
  stopAutoRefresh()
  if (!autoRefreshState.enabled || document.hidden) {
    return
  }
  autoRefreshTimerId = window.setInterval(() => {
    window.dispatchEvent(new CustomEvent('mosdns-log-refresh'))
  }, Math.max(5, autoRefreshState.intervalSeconds) * 1000)
}

function handleAutoRefreshUpdate(event) {
  autoRefreshState = { ...autoRefreshState, ...(event?.detail || {}) }
  startAutoRefresh()
}

function handleVisibilityChange() {
  startAutoRefresh()
}

function triggerGlobalRefresh() {
  window.dispatchEvent(new CustomEvent('mosdns-log-refresh'))
}

async function handleOpenLogFilter(event) {
  const detail = event?.detail || {}
  const text = String(detail.value || '').trim()
  if (!text) {
    return
  }
  await router.push('/logs')
  await nextTick()
  window.dispatchEvent(new CustomEvent('mosdns-open-log-filter-ready', {
    detail: {
      value: text,
      exact: Boolean(detail.exact)
    }
  }))
}

function clearTopNotice() {
  if (topNoticeTimerId) {
    window.clearTimeout(topNoticeTimerId)
    topNoticeTimerId = 0
  }
  topNotice.open = false
  topNotice.message = ''
}

function showTopNotice(payload = {}) {
  const message = String(payload?.message || '').trim()
  if (!message) {
    clearTopNotice()
    return
  }
  const tone = String(payload?.tone || '').toLowerCase() === 'error' ? 'error' : 'success'
  const durationMsRaw = Number(payload?.durationMs || 2400)
  const durationMs = Number.isFinite(durationMsRaw) ? Math.min(8000, Math.max(1200, durationMsRaw)) : 2400
  topNotice.tone = tone
  topNotice.message = message
  topNotice.open = true
  if (topNoticeTimerId) {
    window.clearTimeout(topNoticeTimerId)
  }
  topNoticeTimerId = window.setTimeout(() => {
    topNoticeTimerId = 0
    topNotice.open = false
  }, durationMs)
}

function handleTopNoticeEvent(event) {
  showTopNotice(event?.detail || {})
}

function loadHealth() {
  if (supportsOperation('system.health')) getJSON('/api/v1/system/health').catch(() => {})
}

onMounted(async () => {
  try { await getRuntimeCapabilities() } catch { /* Keep retryable discovery visible and optional children unmounted. */ }
  loadHealth()
  window.addEventListener('mosdns-capabilities-ready', loadHealth)
  startAutoRefresh()
  removeBeforeUnload = installBeforeUnload()
  window.addEventListener('mosdns-auto-refresh-update', handleAutoRefreshUpdate)
  window.addEventListener('mosdns-top-notice', handleTopNoticeEvent)
  window.addEventListener('mosdns-open-log-filter', handleOpenLogFilter)
  document.addEventListener('visibilitychange', handleVisibilityChange)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-capabilities-ready', loadHealth)
  stopAutoRefresh()
  removeBeforeUnload()
  clearTopNotice()
  window.removeEventListener('mosdns-auto-refresh-update', handleAutoRefreshUpdate)
  window.removeEventListener('mosdns-top-notice', handleTopNoticeEvent)
  window.removeEventListener('mosdns-open-log-filter', handleOpenLogFilter)
  document.removeEventListener('visibilitychange', handleVisibilityChange)
})
</script>

<template>
  <div class="rt-shell">
    <aside class="rt-sidebar" aria-label="导航">
      <div class="rt-brand">
        <strong>rtdns</strong>
        <span v-if="versionText" class="rt-brand-version">{{ versionText }}</span>
      </div>
      <nav class="rt-nav">
        <template v-for="module in NAV_MODULES" :key="module.id">
          <router-link
            v-if="module.items.length === 0"
            :to="module.path"
            class="rt-nav-link rt-nav-module"
            :class="{ active: activeModule.id === module.id }"
          >
            {{ module.label }}
          </router-link>
          <template v-else>
            <button
              type="button"
              class="rt-nav-group"
              :class="{ active: activeModule.id === module.id, open: expanded[module.id] }"
              :aria-expanded="expanded[module.id] ? 'true' : 'false'"
              :aria-controls="`rt-nav-items-${module.id}`"
              :data-nav-toggle="module.id"
              @click="toggleModule(module.id)"
            >
              <span>{{ module.label }}</span>
              <span class="rt-nav-caret" aria-hidden="true">{{ expanded[module.id] ? '▾' : '▸' }}</span>
            </button>
            <div v-show="expanded[module.id]" :id="`rt-nav-items-${module.id}`" class="rt-nav-items">
              <router-link
                v-for="item in module.items"
                :key="item.path"
                :to="item.path"
                class="rt-nav-link rt-nav-item"
                active-class="active"
              >
                {{ item.label }}
              </router-link>
            </div>
          </template>
        </template>
      </nav>
    </aside>

    <nav class="rt-mobile-nav" aria-label="导航">
      <div class="rt-mobile-row">
        <template v-for="module in NAV_MODULES" :key="module.id">
          <router-link
            v-if="module.items.length === 0"
            :to="module.path"
            class="rt-mobile-tab"
            :class="{ active: activeModule.id === module.id }"
          >
            {{ module.label }}
          </router-link>
          <button
            v-else
            type="button"
            class="rt-mobile-tab"
            :class="{ active: activeModule.id === module.id }"
            :aria-expanded="expanded[module.id] ? 'true' : 'false'"
            :data-nav-toggle="module.id"
            @click="toggleModule(module.id)"
          >
            {{ module.label }} {{ expanded[module.id] ? '▾' : '▸' }}
          </button>
        </template>
      </div>
      <div v-for="module in expandedModules" :key="`sub-${module.id}`" class="rt-mobile-row rt-mobile-sub">
        <router-link
          v-for="item in module.items"
          :key="item.path"
          :to="item.path"
          class="rt-mobile-tab"
          active-class="active"
        >
          {{ item.label }}
        </router-link>
      </div>
    </nav>

    <div class="rt-main">
      <header class="rt-topbar">
        <h1 class="rt-page-title">{{ pageTitle }}</h1>
        <transition name="top-inline-notice-fade">
          <div v-if="topNotice.open" class="rt-notice" :class="topNotice.tone === 'error' ? 'error' : 'success'" role="status" aria-live="polite">
            {{ topNotice.message }}
          </div>
        </transition>
        <button class="btn secondary rt-refresh-btn" type="button" title="刷新当前页面数据" @click="triggerGlobalRefresh">⟳ 刷新</button>
      </header>

      <div v-if="capabilityState.status !== 'ready'" role="alert" class="panel">
        {{ capabilityState.status === 'pending' ? '正在读取运行时能力' : capabilityState.error }}
        <button v-if="capabilityState.status === 'error'" class="btn secondary" @click="retryRuntimeCapabilities().catch(() => {})">重试能力发现</button>
      </div>

      <main v-if="capabilityState.value" class="main-body">
        <router-view v-slot="{ Component }">
          <section :key="route.path" class="page-shell">
            <component :is="Component" />
          </section>
        </router-view>
      </main>
    </div>

    <ConfirmBubbleHost />
  </div>
</template>
