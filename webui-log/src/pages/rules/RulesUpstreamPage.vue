<script setup>
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, postJSON } from '../../api/http'
import { supportsOperation } from '../../api/runtimeCapabilities'
import CapabilityBoundary from '../../components/CapabilityBoundary.vue'
import UpstreamManager from '../../components/UpstreamManager.vue'
import SystemOverridesPanel from '../../components/system/SystemOverridesPanel.vue'
import { settingsSources } from '../../utils/nativeSettings'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'

defineProps({
  // The special-groups sub-item mounts the same page until the second part splits it.
  specialGroups: { type: Boolean, default: false }
})

const applyingOverrides = ref(false)
const overrides = reactive({ socks5: '', ecs: '' })
// Where each value comes from ('yaml' | 'settings') and the saved values.
const overrideSources = reactive({ socks5: '', ecs: '' })
const overridesSaved = reactive({ socks5: '', ecs: '' })

async function loadOverrides() {
  if (!supportsOperation('system.global_overrides')) return
  const data = await getJSON('/api/v1/overrides')
  overrides.socks5 = String(data?.socks5 || '')
  overrides.ecs = String(data?.ecs || '')
  overridesSaved.socks5 = overrides.socks5
  overridesSaved.ecs = overrides.ecs
  try {
    const marks = settingsSources(await getJSON('/api/v1/settings/effective'))
    overrideSources.socks5 = marks.socks5
    overrideSources.ecs = marks.ecs
  } catch {
    overrideSources.socks5 = ''
    overrideSources.ecs = ''
  }
}

async function reloadOverrides() {
  try {
    await loadOverrides()
  } catch (error) {
    setError(`读取覆盖配置失败: ${error.message}`)
  }
}

async function restoreOverride(field) {
  if (!supportsOperation('system.global_overrides')) return
  clearTopNotice()
  applyingOverrides.value = true
  try {
    // An empty value returns that field to the configuration file's value.
    await postJSON('/api/v1/overrides', {
      socks5: field === 'socks5' ? '' : overridesSaved.socks5,
      ecs: field === 'ecs' ? '' : overridesSaved.ecs,
      replacements: []
    })
    setSuccess('已恢复为配置文件里的值')
    await loadOverrides()
  } catch (error) {
    setError(`恢复默认失败: ${error.message}`)
  } finally {
    applyingOverrides.value = false
  }
}

async function saveOverrides() {
  if (!supportsOperation('system.global_overrides')) return
  clearTopNotice()
  applyingOverrides.value = true
  try {
    // Applied at once; replacements do not exist in this runtime.
    await postJSON('/api/v1/overrides', {
      socks5: String(overrides.socks5 || '').trim(),
      ecs: String(overrides.ecs || '').trim(),
      replacements: []
    })
    setSuccess('覆盖配置已保存并立即生效')
    await loadOverrides()
  } catch (error) {
    setError(`保存覆盖配置失败: ${error.message}`)
  } finally {
    applyingOverrides.value = false
  }
}

onMounted(() => {
  reloadOverrides()
  window.addEventListener('mosdns-log-refresh', reloadOverrides)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', reloadOverrides)
})
</script>

<template>
  <p v-if="specialGroups" class="muted rt-page-note" role="note">第一段临时位置：专属分流组在下方上游页面的"专属分流组 › 管理"里编辑，第二段拆成独立页面。</p>
  <UpstreamManager />
  <CapabilityBoundary operation="system.global_overrides">
    <SystemOverridesPanel
      :applying-overrides="applyingOverrides"
      :overrides="overrides"
      native
      :sources="overrideSources"
      @load-overrides="reloadOverrides"
      @save-overrides="saveOverrides"
      @restore-override="restoreOverride"
    />
  </CapabilityBoundary>
</template>
