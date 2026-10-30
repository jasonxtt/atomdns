<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { getText, postJSON } from '../../api/http'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { LIST_LINE_CAP, canSaveList, filterLines, readList, sameRules, showUrl, splitRules } from '../../utils/ruleList'
import { clearTopNotice, setSuccess } from '../../utils/notice'
import { clearDirty, setDirty } from '../../utils/unsaved'

// The one editor for every manual list (white/grey/black list, DDNS domains, direct IPs,
// client IPs, redirects, the fakeip filters and the lists of special groups): row count,
// search (display only), save with the server's own error text, a guard against saving a
// list that was cut off, and the unsaved-changes warning.
const props = defineProps({
  tag: { type: String, required: true },
  title: { type: String, required: true },
  hint: { type: String, default: '' },
  placeholder: { type: String, default: '每行一个条目' },
  cap: { type: Number, default: LIST_LINE_CAP }
})

const uid = `list:${props.tag}:${Math.random().toString(36).slice(2)}`

const content = ref('')
// What the server held when the editor last read it (canonical text).
const original = ref('')
const truncated = ref(false)
// The save went through but the re-read failed: the content on the server is not known.
const uncertain = ref(false)
const missing = ref(false)
// The list was read completely at least once; until then nothing can be saved, because a
// save replaces the whole list with what the editor holds.
const loaded = ref(false)
const loading = ref(false)
const saving = ref(false)
const error = ref('')
const notice = ref('')
const search = ref('')

const canRead = computed(() => supportsOperation('rules.local.read'))
const canWrite = computed(() => supportsOperation('rules.local.manage'))
const dirty = computed(() => !sameRules(content.value, original.value))
const lineCount = computed(() => splitRules(content.value).length)
const matches = computed(() => filterLines(content.value, search.value))
const searching = computed(() => search.value.trim() !== '')
const saveEnabled = computed(() => canWrite.value && loaded.value && !missing.value && canSaveList({ truncated: truncated.value, loading: loading.value, saving: saving.value, uncertain: uncertain.value, dirty: dirty.value }))

watch([dirty, uncertain], () => setDirty(uid, dirty.value || uncertain.value), { immediate: true })

async function fetchCanonical() {
  return readList(await getText(showUrl(props.tag, props.cap)), props.cap)
}

function adopt(read) {
  original.value = read.text
  content.value = read.text
  truncated.value = read.truncated
}

async function load() {
  if (!canRead.value) return
  loading.value = true
  error.value = ''
  try {
    adopt(await fetchCanonical())
    uncertain.value = false
    missing.value = false
    loaded.value = true
  } catch (failure) {
    if (failure.status === 404) missing.value = true
    else error.value = `读取失败: ${failure.message}`
  } finally {
    loading.value = false
  }
}

// An auto refresh must not replace what the user is typing.
function refresh() {
  if (dirty.value || uncertain.value || saving.value) return
  return load()
}

async function save() {
  if (!saveEnabled.value) return
  clearTopNotice()
  error.value = ''
  notice.value = ''
  saving.value = true
  try {
    if (uncertain.value) {
      // A POST is never repeated blindly: read what the server holds first.
      const read = await fetchCanonical()
      uncertain.value = false
      original.value = read.text
          truncated.value = read.truncated
      if (sameRules(content.value, read.text)) {
        content.value = read.text
        setSuccess(`“${props.title}”已保存`)
      } else {
        notice.value = '服务器上的内容与这里的编辑不同，已保留你的编辑，请确认后再保存。'
      }
      return
    }
    const values = splitRules(content.value)
    try {
      await postJSON(`/plugins/${encodeURIComponent(props.tag)}/post`, { values })
    } catch (failure) {
      error.value = `保存失败: ${failure.message}`
      return
    }
    // The server skips rules it rejects, so only a re-read says what was kept.
    try {
      const read = await fetchCanonical()
      const submitted = values.join('\n')
      adopt(read)
      if (read.truncated) notice.value = '名单超过显示上限，已按服务器内容更新；页面上不能再保存这份名单。'
      else if (!sameRules(read.text, submitted)) notice.value = '部分规则未被服务器接受，已按服务器内容更新。'
      else setSuccess(`“${props.title}”已保存，共 ${read.count} 条`)
    } catch (failure) {
      uncertain.value = true
      error.value = `已提交，但重新读取失败（${failure.message}）；服务器上的内容未确认，请再点保存来确认。`
    }
  } catch (failure) {
    error.value = `保存失败: ${failure.message}`
  } finally {
    saving.value = false
  }
}

function revert() {
  content.value = original.value
  notice.value = ''
  error.value = ''
}

onMounted(() => {
  load()
  window.addEventListener('atomdns-log-refresh', refresh)
})
onBeforeUnmount(() => {
  window.removeEventListener('atomdns-log-refresh', refresh)
  clearDirty(uid)
})
</script>

<template>
  <section class="panel rt-list-editor" :data-list="tag">
    <header class="rt-list-head">
      <div class="rt-list-title">
        <h3>{{ title }}</h3>
        <span class="muted rt-list-count" data-list-count>{{ loading ? '读取中…' : truncated ? `超过 ${cap.toLocaleString()} 条` : `共 ${lineCount.toLocaleString()} 条` }}</span>
        <span v-if="dirty" class="rt-flag-mark" data-list-dirty>未保存</span>
      </div>
      <input v-if="!missing && canRead" v-model="search" class="rt-setting-input rt-list-search" type="search" placeholder="搜索（只过滤显示）" :aria-label="`搜索${title}`" />
    </header>
    <p v-if="hint" class="muted rt-list-hint">{{ hint }}</p>
    <p v-if="!canRead" class="muted" role="note">{{ operationReason('rules.local.read') }}</p>
    <p v-else-if="missing" class="muted" role="note" data-list-missing>当前配置没有这份名单。</p>
    <template v-else>
      <p v-if="truncated" class="rt-setting-error" role="alert" data-list-truncated>
        名单超过 {{ cap.toLocaleString() }} 条，页面不能完整显示，下面只显示了前面一部分；为避免丢失后面的条目，这份名单不能在页面上保存，请直接编辑名单文件。
      </p>
      <ul v-if="searching" class="mono rt-list-matches" data-list-matches>
        <li v-if="matches.length === 0" class="muted">没有匹配的条目</li>
        <li v-for="(line, index) in matches.slice(0, 300)" :key="index">{{ line }}</li>
        <li v-if="matches.length > 300" class="muted">只显示前 300 条，共匹配 {{ matches.length.toLocaleString() }} 条</li>
      </ul>
      <textarea
        v-show="!searching"
        v-model="content"
        class="list-editor rt-list-text"
        spellcheck="false"
        :disabled="loading || saving"
        :readonly="!canWrite || !loaded || truncated"
        :placeholder="placeholder"
        :aria-label="title"
      />
      <p v-if="searching" class="muted rt-list-hint">搜索时只显示匹配的行；清空搜索框后可继续编辑，编辑内容不会丢失。</p>
      <p v-if="!canWrite" class="muted" role="note">{{ operationReason('rules.local.manage') }}</p>
      <p v-if="!loaded && !loading" class="muted" role="note" data-list-unread>这份名单还没有成功读取，读取成功之前不能保存，以免用空内容覆盖原有的名单。<button class="btn-link" type="button" @click="load">重新读取</button></p>
      <p v-if="error" class="rt-setting-error" role="alert" data-list-error>{{ error }}</p>
      <p v-if="notice" class="muted" role="status" data-list-notice>{{ notice }}</p>
      <div class="actions rt-list-actions">
        <button class="btn primary" type="button" :disabled="!saveEnabled" @click="save">{{ saving ? '保存中…' : uncertain ? '确认并保存' : '保存' }}</button>
        <button class="btn secondary" type="button" :disabled="saving || loading || (!dirty && !uncertain)" @click="uncertain ? load() : revert()">还原</button>
      </div>
    </template>
  </section>
</template>
