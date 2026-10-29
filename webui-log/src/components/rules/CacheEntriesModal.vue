<script setup>
import { onBeforeUnmount, onMounted, reactive } from 'vue'
import { capabilityFetch } from '../../api/runtimeCapabilities'
import AppIcon from '../AppIcon.vue'

// The entries of one cache, newest page first, searchable and loaded in pages.
const props = defineProps({ cache: { type: Object, required: true } })
defineEmits(['close'])

const view = reactive({ query: '', entries: [], total: 0, offset: 0, limit: 100, hasMore: true, loading: false, loadingMore: false, error: '' })
let searchTimer = 0

function parseEntries(text, start) {
  const chunks = String(text || '').trim() ? String(text).trim().split('----- Cache Entry -----').filter((chunk) => chunk.trim() !== '') : []
  return chunks.map((chunk, index) => {
    const question = chunk.match(/;; QUESTION SECTION:\s*;\s*([^\s]+)/)
    const domainSet = chunk.match(/DomainSet:\s*(.+)/)
    let title = question ? question[1].replace(/\.$/, '') : `条目 ${start + index + 1}`
    if (domainSet) title += ` [${domainSet[1].trim()}]`
    const split = chunk.indexOf('DNS Message:')
    const meta = (split >= 0 ? chunk.slice(0, split) : chunk).trim().split('\n').map((line) => line.match(/^([^:]+):\s*(.*)$/)).filter(Boolean).map((parts) => ({ key: parts[1].trim(), value: parts[2].trim() }))
    return { key: `${title}-${start + index}`, title, meta, message: split >= 0 ? chunk.slice(split).trim() : '没有 DNS 报文' }
  })
}

async function fetchEntries(append = false) {
  if (append) view.loadingMore = true
  else {
    view.loading = true
    view.error = ''
  }
  try {
    const params = new URLSearchParams({ q: view.query, offset: String(view.offset), limit: String(view.limit) })
    const response = await capabilityFetch(`/plugins/${encodeURIComponent(props.cache.tag)}/show?${params.toString()}`)
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    const total = Number.parseInt(response.headers.get('X-Total-Count') || '', 10) || 0
    const next = parseEntries(await response.text(), view.offset)
    view.entries = append ? [...view.entries, ...next] : next
    view.total = total
    view.offset += next.length
    view.hasMore = total > 0 ? view.entries.length < total : next.length >= view.limit
  } catch (error) {
    if (!append) view.entries = []
    view.error = `读取失败: ${error.message}`
    view.hasMore = false
  } finally {
    view.loading = false
    view.loadingMore = false
  }
}

function search() {
  window.clearTimeout(searchTimer)
  searchTimer = window.setTimeout(() => {
    Object.assign(view, { offset: 0, entries: [], total: 0, hasMore: true })
    fetchEntries(false)
  }, 350)
}

onMounted(() => fetchEntries(false))
onBeforeUnmount(() => window.clearTimeout(searchTimer))
</script>

<template>
  <Teleport to="body">
    <div class="modal-mask modal-mask-center" @click.self="$emit('close')">
      <section class="panel data-view-modal detail-modal" role="dialog" :aria-label="`${cache.name} 的条目`">
        <header class="panel-header">
          <div>
            <h3>{{ cache.name }}</h3>
            <p class="muted">已显示 {{ view.entries.length.toLocaleString() }}{{ view.total > 0 ? ` / ${view.total.toLocaleString()}` : '' }} 条</p>
          </div>
          <button class="rt-row-icon" type="button" aria-label="关闭" title="关闭" @click="$emit('close')"><AppIcon name="x" :size="18" /></button>
        </header>
        <div class="data-view-search"><input v-model="view.query" placeholder="搜索…" aria-label="搜索缓存条目" @input="search" /></div>
        <p v-if="view.error" class="muted">{{ view.error }}</p>
        <div class="cache-entry-list">
          <p v-if="view.loading" class="empty">读取中…</p>
          <p v-else-if="view.entries.length === 0" class="empty">没有匹配的条目</p>
          <details v-for="item in view.entries" :key="item.key" class="cache-entry">
            <summary>{{ item.title }}</summary>
            <div class="table-wrap cache-meta-wrap">
              <table><tbody><tr v-for="(meta, index) in item.meta" :key="`${item.key}-${index}`"><td>{{ meta.key }}</td><td class="mono">{{ meta.value }}</td></tr></tbody></table>
            </div>
            <pre class="mono cache-pre">{{ item.message }}</pre>
          </details>
        </div>
        <div class="actions log-more">
          <button class="btn primary" type="button" :disabled="view.loading || view.loadingMore || !view.hasMore" @click="fetchEntries(true)">{{ view.loadingMore ? '读取中…' : '加载更多' }}</button>
        </div>
      </section>
    </div>
  </Teleport>
</template>
