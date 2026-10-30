<script setup>
import { ref } from 'vue'
import { capabilityFetch } from '../../api/runtimeCapabilities'
import { supportsOperation } from '../../api/runtimeCapabilities'
import CapabilityBoundary from '../../components/CapabilityBoundary.vue'
import SettingRow from '../../components/settings/SettingRow.vue'
import { clearTopNotice, setError, setSuccess } from '../../utils/notice'

const CONFIG_DIR_KEY = 'atomdns-config-dir'
const BACKUP_FILENAME = 'atomdns_backup.zip'

function readConfigDir() {
  try { return localStorage.getItem(CONFIG_DIR_KEY) || '' } catch { return '' }
}

const configDir = ref(readConfigDir())
const backingUp = ref(false)

function saveConfigDir() {
  try { localStorage.setItem(CONFIG_DIR_KEY, String(configDir.value || '').trim()) } catch { /* kept for this page only */ }
}

async function backupConfig() {
  if (!supportsOperation('system.config_management')) return
  const dir = String(configDir.value || '').trim()
  if (!dir) {
    setError('请先输入 AtomDNS 的配置目录')
    return
  }
  clearTopNotice()
  saveConfigDir()
  backingUp.value = true
  try {
    const response = await capabilityFetch('/api/v1/config/export', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ dir })
    })
    if (!response.ok) {
      const text = await response.text().catch(() => '')
      throw new Error(text || `HTTP ${response.status} ${response.statusText}`)
    }
    const blob = await response.blob()
    const downloadUrl = window.URL.createObjectURL(blob)
    const link = document.createElement('a')
    link.style.display = 'none'
    link.href = downloadUrl
    link.download = BACKUP_FILENAME
    document.body.appendChild(link)
    link.click()
    document.body.removeChild(link)
    window.URL.revokeObjectURL(downloadUrl)
    setSuccess('备份文件下载已开始')
  } catch (error) {
    setError(`备份失败: ${error.message}`)
  } finally {
    backingUp.value = false
  }
}
</script>

<template>
  <!-- One tile of the 配置与UI group (mounted by SystemUpdateSection). -->
  <CapabilityBoundary operation="system.config_management">
    <SettingRow label="备份配置" layout="form" description="把配置目录打包成 zip 下载（不含 backup、update 目录），目录须是 AtomDNS 实际使用的配置目录">
      <input v-model="configDir" class="rt-setting-input" placeholder="/etc/atomdns" aria-label="配置目录" @change="saveConfigDir" />
      <button class="btn tiny primary" :disabled="backingUp" @click="backupConfig">{{ backingUp ? '备份中...' : '备份' }}</button>
    </SettingRow>
  </CapabilityBoundary>
</template>
