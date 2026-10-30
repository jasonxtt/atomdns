<script setup>
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, postJSON } from '../../api/http'
import { operationReason, supportsOperation } from '../../api/runtimeCapabilities'
import { NATIVE_OVERRIDES_NOTE, isModified, settingsSources } from '../../utils/nativeSettings'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'
import SettingRow from '../settings/SettingRow.vue'

// The SOCKS5 proxy and the ECS address, applied at once; an empty value returns the
// field to the configuration file's value.
const applying = ref(false)
// Both fields were read; saving or restoring posts both fields, so it waits for that.
const loaded = ref(false)
const overrides = reactive({ socks5: '', ecs: '' })
// Where each value comes from ('yaml' | 'settings') and the saved values.
const sources = reactive({ socks5: '', ecs: '' })
const saved = reactive({ socks5: '', ecs: '' })

async function load() {
  if (!supportsOperation('system.global_overrides')) return
  try {
    const data = await getJSON('/api/v1/overrides')
    overrides.socks5 = String(data?.socks5 || '')
    overrides.ecs = String(data?.ecs || '')
    saved.socks5 = overrides.socks5
    saved.ecs = overrides.ecs
    loaded.value = true
    try {
      const marks = settingsSources(await getJSON('/api/v1/settings/effective'))
      sources.socks5 = marks.socks5
      sources.ecs = marks.ecs
    } catch {
      sources.socks5 = ''
      sources.ecs = ''
    }
  } catch (error) {
    setError(`读取覆盖配置失败: ${error.message}`)
  }
}

async function restoreOverride(field) {
  if (!supportsOperation('system.global_overrides') || !loaded.value) return
  clearTopNotice()
  applying.value = true
  try {
    // An empty value returns that field to the configuration file's value.
    await postJSON('/api/v1/overrides', {
      socks5: field === 'socks5' ? '' : saved.socks5,
      ecs: field === 'ecs' ? '' : saved.ecs,
      replacements: []
    })
    setSuccess('已恢复为配置文件里的值')
    await load()
  } catch (error) {
    setError(`恢复默认失败: ${error.message}`)
  } finally {
    applying.value = false
  }
}

async function saveOverrides() {
  if (!supportsOperation('system.global_overrides') || !loaded.value) return
  clearTopNotice()
  applying.value = true
  try {
    // Applied at once; the body keeps the frozen `replacements` field, which stays empty.
    await postJSON('/api/v1/overrides', {
      socks5: String(overrides.socks5 || '').trim(),
      ecs: String(overrides.ecs || '').trim(),
      replacements: []
    })
    setSuccess('覆盖配置已保存并立即生效')
    await load()
  } catch (error) {
    setError(`保存覆盖配置失败: ${error.message}`)
  } finally {
    applying.value = false
  }
}

onMounted(() => {
  load()
  window.addEventListener('atomdns-log-refresh', load)
})
onBeforeUnmount(() => window.removeEventListener('atomdns-log-refresh', load))
</script>

<template>
  <section class="panel rt-setting-section rt-setting-tiles rt-setting-tiles-compact rt-setting-joined" data-overrides>
    <h3 class="rt-setting-section-title">代理与 ECS</h3>
    <p v-if="!supportsOperation('system.global_overrides')" class="muted rt-setting-note" role="note">{{ operationReason('system.global_overrides') }}</p>
    <template v-else>
      <p class="muted rt-setting-note rt-overrides-note" data-native-overrides-note>{{ NATIVE_OVERRIDES_NOTE }}</p>
      <div class="rt-setting-grid">
        <SettingRow label="SOCKS5 代理" layout="form" description="上游走代理时使用的地址，格式 host:port。">
          <span v-if="isModified(sources.socks5)" class="rt-flag-mark" data-override-modified="socks5">已修改</span>
          <input v-model="overrides.socks5" class="rt-setting-input" placeholder="host:port" aria-label="SOCKS5 代理" />
          <button v-if="isModified(sources.socks5)" class="btn secondary" type="button" :disabled="applying || !loaded" @click="restoreOverride('socks5')">恢复默认</button>
        </SettingRow>
        <SettingRow label="ECS 地址" layout="form" description="向国外上游查询时携带的客户端子网地址（IPv4 或 IPv6）。">
          <span v-if="isModified(sources.ecs)" class="rt-flag-mark" data-override-modified="ecs">已修改</span>
          <input v-model="overrides.ecs" class="rt-setting-input" placeholder="IPv4 / IPv6" aria-label="ECS 地址" />
          <button v-if="isModified(sources.ecs)" class="btn secondary" type="button" :disabled="applying || !loaded" @click="restoreOverride('ecs')">恢复默认</button>
        </SettingRow>
      </div>
      <div class="actions rt-list-actions">
        <button class="btn primary" type="button" :disabled="applying || !loaded" @click="saveOverrides">{{ applying ? '保存中…' : '保存并应用' }}</button>
        <button class="btn secondary" type="button" :disabled="applying" @click="load">读取当前</button>
      </div>
    </template>
  </section>
</template>
