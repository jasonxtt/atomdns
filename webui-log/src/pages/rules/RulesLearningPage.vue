<script setup>
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, postJSON } from '../../api/http'
import { supportsOperation } from '../../api/runtimeCapabilities'
import CapabilityBoundary from '../../components/CapabilityBoundary.vue'
import DataManagementManager from '../../components/DataManagementManager.vue'
import SystemDomainGenerationPanel from '../../components/system/SystemDomainGenerationPanel.vue'
import { openConfirm } from '../../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'

const domainGenerationLoading = ref(false)
const domainGenerationSettings = reactive({
  enabled: true,
  remember_direct: true,
  remember_proxy: true,
  no_v4: true,
  no_v6: true
})

const domainGenerationProfiles = [
  { key: 'enabled', name: '总开关', desc: '统一控制域名表生成' },
  { key: 'remember_direct', name: '记忆直连', desc: '生成直连域名表' },
  { key: 'remember_proxy', name: '记忆代理', desc: '生成代理域名表' },
  { key: 'no_v4', name: '记忆无 V4', desc: '生成无 IPv4 域名表' },
  { key: 'no_v6', name: '记忆无 V6', desc: '生成无 IPv6 域名表' }
]

function applyDomainGenerationSettings(next) {
  domainGenerationSettings.enabled = Boolean(next?.enabled)
  domainGenerationSettings.remember_direct = Boolean(next?.remember_direct)
  domainGenerationSettings.remember_proxy = Boolean(next?.remember_proxy)
  domainGenerationSettings.no_v4 = Boolean(next?.no_v4)
  domainGenerationSettings.no_v6 = Boolean(next?.no_v6)
}

function buildDomainGenerationPayload(key, checked) {
  if (key === 'enabled') {
    return {
      enabled: checked,
      remember_direct: checked,
      remember_proxy: checked,
      no_v4: checked,
      no_v6: checked
    }
  }

  const payload = { [key]: checked }
  if (checked && !domainGenerationSettings.enabled) {
    payload.enabled = true
  }
  return payload
}

async function toggleDomainGeneration(profile, checked) {
  if (!supportsOperation('system.domain_generation')) return
  if (!profile?.key) {
    return
  }
  clearTopNotice()
  try {
    if (!checked) {
      const confirmed = await openConfirm(
        `关闭“${profile.name}”后，会停止新增并清空当前已生成数据，确认继续？`,
        {
          tone: 'danger',
          confirmText: '关闭并清空'
        }
      )
      if (!confirmed) {
        return
      }
    }
    domainGenerationLoading.value = true
    const payload = buildDomainGenerationPayload(profile.key, checked)
    const response = await postJSON('/api/v1/domain-generation', payload)
    applyDomainGenerationSettings(response?.settings || response)
    setSuccess(`“${profile.name}” 已${checked ? '启用' : '关闭'}`)
  } catch (error) {
    setError(`切换“${profile.name}”失败: ${error.message}`)
  } finally {
    domainGenerationLoading.value = false
  }
}

async function loadDomainGenerationSettings() {
  if (!supportsOperation('system.domain_generation')) return
  try {
    applyDomainGenerationSettings(await getJSON('/api/v1/domain-generation'))
  } catch (error) {
    setError(`读取域名表自生成设置失败: ${error.message}`)
  }
}

onMounted(() => {
  loadDomainGenerationSettings()
  window.addEventListener('mosdns-log-refresh', loadDomainGenerationSettings)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', loadDomainGenerationSettings)
})
</script>

<template>
  <CapabilityBoundary operation="system.domain_generation">
    <SystemDomainGenerationPanel
      :domain-generation-profiles="domainGenerationProfiles"
      :domain-generation-loading="domainGenerationLoading"
      :domain-generation-settings="domainGenerationSettings"
      @toggle-domain-generation="toggleDomainGeneration"
    />
  </CapabilityBoundary>
  <DataManagementManager mode="learning" />
</template>
