<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { deleteRequest, getJSON, postJSON } from '../../api/http'
import { flagsForPage, isValidFlagList, optionLabel, runFlagChange } from '../../api/flags'
import { supportsOperation } from '../../api/runtimeCapabilities'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'
import SettingRow from '../settings/SettingRow.vue'

// The switches of one sub-item of 规则设置: the flags the configuration declares for
// `page`, in declaration order. A click is applied at once; while one change is being
// applied the other switches of the page wait.
const props = defineProps({ page: { type: String, required: true } })

const flags = ref([])
const loaded = ref(false)
const loadError = ref('')
// The key being applied; every switch of the page is disabled meanwhile.
const busyKey = ref('')

const pageFlags = computed(() => flagsForPage(flags.value, props.page))
const enabled = computed(() => supportsOperation('flags.manage'))

async function load() {
  if (!enabled.value) return
  try {
    const data = await getJSON('/api/v1/flags')
    if (!isValidFlagList(data)) throw new Error('开关列表格式无法识别')
    flags.value = data.flags
    loadError.value = ''
  } catch (error) {
    loadError.value = `读取开关失败: ${error.message}`
  } finally {
    loaded.value = true
  }
}

async function apply(flag, request, event) {
  if (busyKey.value) return
  clearTopNotice()
  busyKey.value = flag.key
  try {
    const outcome = await runFlagChange(flag, request)
    if (outcome.ok) setSuccess(outcome.message)
    else setError(outcome.message)
    if (outcome.reload) await load()
  } finally {
    busyKey.value = ''
    // A rejected click leaves the checkbox where the user put it; the flag decides.
    if (event?.target && 'checked' in event.target) {
      const current = flags.value.find((item) => item.key === flag.key)
      if (current) event.target.checked = current.value === true
    }
  }
}

function setValue(flag, value, event) {
  if (value === flag.value) return
  return apply(flag, () => postJSON(`/api/v1/flags/${encodeURIComponent(flag.key)}`, { value }), event)
}

function restore(flag) {
  return apply(flag, () => deleteRequest(`/api/v1/flags/${encodeURIComponent(flag.key)}`))
}

function valueText(flag) {
  return flag.kind === 'choice' ? optionLabel(flag, flag.value) : flag.value ? '开启' : '关闭'
}

onMounted(() => {
  load()
  window.addEventListener('mosdns-log-refresh', load)
})
onBeforeUnmount(() => window.removeEventListener('mosdns-log-refresh', load))
</script>

<template>
  <section v-if="!enabled || loadError || pageFlags.length > 0" class="panel rt-setting-section" :data-flags-page="page">
    <h3 class="rt-setting-section-title">开关</h3>
    <p v-if="!enabled" class="muted rt-setting-note" role="note">当前配置没有可修改的功能开关。</p>
    <p v-else-if="loadError" class="rt-setting-error" role="alert">{{ loadError }}</p>
    <SettingRow v-for="flag in pageFlags" :key="flag.key" :label="flag.label" :description="flag.description" :data-flag="flag.key">
      <span v-if="flag.modified" class="rt-flag-mark" :data-flag-modified="flag.key">已修改</span>
      <span v-if="busyKey === flag.key" class="rt-setting-note" data-flag-busy>应用中…</span>
      <div v-if="flag.kind === 'choice'" class="rt-segmented" role="group" :aria-label="flag.label">
        <button
          v-for="option in flag.options"
          :key="option.value"
          type="button"
          :class="{ active: flag.value === option.value }"
          :aria-pressed="flag.value === option.value"
          :disabled="Boolean(busyKey)"
          @click="setValue(flag, option.value, $event)"
        >{{ option.label }}</button>
      </div>
      <label v-else class="switch" :title="valueText(flag)">
        <input
          type="checkbox"
          :checked="flag.value === true"
          :disabled="Boolean(busyKey)"
          :aria-label="flag.label"
          @change="setValue(flag, $event.target.checked, $event)"
        />
        <span class="slider"></span>
      </label>
      <button v-if="flag.modified" class="btn secondary" type="button" :disabled="Boolean(busyKey)" @click="restore(flag)">恢复默认</button>
    </SettingRow>
  </section>
</template>
