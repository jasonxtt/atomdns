<script setup>
import { capabilityState, getRuntimeCapabilities, retryRuntimeCapabilities, supportsOperation } from './api/runtimeCapabilities'
import { computed, nextTick, onBeforeUnmount, onMounted, reactive } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { getJSON } from './api/http'
import ConfirmBubbleHost from './components/ConfirmBubbleHost.vue'
import { NAV_MODULES } from './navigation'
import { loadAutoRefreshState } from './utils/autoRefresh'

const route = useRoute()
const router = useRouter()

// A module owns `/<id>` and everything below it (`#/logs/diagnostic` keeps 日志 highlighted).
const activeModule = computed(() => NAV_MODULES.find(module => route.path === `/${module.id}` || route.path.startsWith(`/${module.id}/`)) || NAV_MODULES[0])
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
  window.addEventListener('mosdns-auto-refresh-update', handleAutoRefreshUpdate)
  window.addEventListener('mosdns-top-notice', handleTopNoticeEvent)
  window.addEventListener('mosdns-open-log-filter', handleOpenLogFilter)
  document.addEventListener('visibilitychange', handleVisibilityChange)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-capabilities-ready', loadHealth)
  stopAutoRefresh()
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
            <div class="rt-nav-group" :class="{ active: activeModule.id === module.id }">{{ module.label }}</div>
            <router-link
              v-for="item in module.items"
              :key="item.path"
              :to="item.path"
              class="rt-nav-link rt-nav-item"
              active-class="active"
            >
              {{ item.label }}
            </router-link>
          </template>
        </template>
      </nav>
    </aside>

    <nav class="rt-mobile-nav" aria-label="导航">
      <div class="rt-mobile-row">
        <router-link
          v-for="module in NAV_MODULES"
          :key="module.id"
          :to="module.path"
          class="rt-mobile-tab"
          :class="{ active: activeModule.id === module.id }"
        >
          {{ module.label }}
        </router-link>
      </div>
      <div v-if="activeModule.items.length > 0" class="rt-mobile-row rt-mobile-sub">
        <router-link
          v-for="item in activeModule.items"
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
