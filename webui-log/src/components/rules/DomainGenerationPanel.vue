<script setup>
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, postJSON } from '../../api/http'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { openConfirm } from '../../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'
import SettingRow from '../settings/SettingRow.vue'

// The learning master switch and the four categories (`/api/v1/domain-generation`).
// Turning one off stops the learning and clears what it learned, so it asks first.
const PROFILES = [
  { key: 'enabled', name: '学习总开关', desc: '统一控制下面四类学习。' },
  { key: 'remember_direct', name: '直连域名', desc: '记住走直连的域名，下次直接直连。' },
  { key: 'remember_proxy', name: '代理域名', desc: '记住要走代理的域名，下次直接走代理。' },
  { key: 'no_v4', name: '无 IPv4 域名', desc: '记住没有 IPv4 地址的域名，A 请求直接返回空。' },
  { key: 'no_v6', name: '无 IPv6 域名', desc: '记住没有 IPv6 地址的域名，AAAA 请求直接返回空。' }
]

const loading = ref(false)
const settings = reactive({ enabled: true, remember_direct: true, remember_proxy: true, no_v4: true, no_v6: true })

function adopt(next) {
  for (const { key } of PROFILES) settings[key] = Boolean(next?.[key])
}

function payloadFor(key, checked) {
  if (key === 'enabled') return Object.fromEntries(PROFILES.map((profile) => [profile.key, checked]))
  return { [key]: checked, ...(checked && !settings.enabled ? { enabled: true } : {}) }
}

async function load() {
  if (!supportsOperation('system.domain_generation')) return
  try {
    adopt(await getJSON('/api/v1/domain-generation'))
  } catch (error) {
    setError(`读取域名学习设置失败: ${error.message}`)
  }
}

async function toggle(profile, event) {
  if (loading.value || !supportsOperation('system.domain_generation')) return
  const checked = event.target.checked
  clearTopNotice()
  try {
    if (!checked && !(await openConfirm(`关闭“${profile.name}”后，会停止新增并清空已学到的数据，确认继续？`, { tone: 'danger', confirmText: '关闭并清空' }))) {
      event.target.checked = true
      return
    }
    loading.value = true
    const response = await postJSON('/api/v1/domain-generation', payloadFor(profile.key, checked))
    adopt(response?.settings || response)
    setSuccess(`“${profile.name}”已${checked ? '开启' : '关闭'}`)
  } catch (error) {
    setError(`切换“${profile.name}”失败: ${error.message}`)
    event.target.checked = Boolean(settings[profile.key])
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  load()
  window.addEventListener('mosdns-log-refresh', load)
})
onBeforeUnmount(() => window.removeEventListener('mosdns-log-refresh', load))
</script>

<template>
  <section class="panel rt-setting-section" data-domain-generation>
    <h3 class="rt-setting-section-title">学习项目</h3>
    <p v-if="!supportsOperation('system.domain_generation')" class="muted rt-setting-note" role="note">{{ operationReason('system.domain_generation') }}</p>
    <template v-else>
      <SettingRow v-for="profile in PROFILES" :key="profile.key" :label="profile.name" :description="profile.desc">
        <label class="switch">
          <input type="checkbox" :checked="Boolean(settings[profile.key])" :disabled="loading" :aria-label="profile.name" @change="toggle(profile, $event)" />
          <span class="slider"></span>
        </label>
      </SettingRow>
    </template>
  </section>
</template>
