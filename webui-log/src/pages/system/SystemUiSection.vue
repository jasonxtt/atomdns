<script setup>
import { computed, inject, reactive } from 'vue'
import SettingRow from '../../components/settings/SettingRow.vue'
import SettingSection from '../../components/settings/SettingSection.vue'
import { loadAutoRefreshState, saveAutoRefreshState } from '../../utils/autoRefresh'
import { setSuccess } from '../../utils/notice'
import { THEME_LABELS, THEME_MODES } from '../../utils/theme'

const theme = inject('theme')
const themeMode = computed(() => theme.mode)
const autoRefresh = reactive(loadAutoRefreshState())

function changeTheme(mode) {
  theme.setMode(mode)
}

function emitAutoRefreshSettings() {
  Object.assign(autoRefresh, saveAutoRefreshState(autoRefresh))
  setSuccess(`自动刷新已${autoRefresh.enabled ? `开启（${autoRefresh.intervalSeconds}秒）` : '关闭'}`)
}

function onAutoRefreshToggle(event) {
  autoRefresh.enabled = Boolean(event?.target?.checked)
  emitAutoRefreshSettings()
}
</script>

<template>
  <SettingSection id="ui" title="界面">
    <SettingRow label="主题" description="跟随系统时随操作系统切换深浅色；只保存在当前浏览器" data-theme-panel>
      <div class="rt-segmented" role="radiogroup" aria-label="主题">
        <button
          v-for="mode in THEME_MODES"
          :key="mode"
          type="button"
          role="radio"
          :aria-checked="themeMode === mode"
          :class="{ active: themeMode === mode }"
          @click="changeTheme(mode)"
        >
          {{ THEME_LABELS[mode] }}
        </button>
      </div>
    </SettingRow>
    <SettingRow label="自动刷新" description="按间隔自动刷新当前页面的数据；只保存在当前浏览器">
      <label class="switch switch-table">
        <input type="checkbox" :checked="autoRefresh.enabled" aria-label="自动刷新" @change="onAutoRefreshToggle" />
        <span class="slider"></span>
      </label>
      <input v-model.number="autoRefresh.intervalSeconds" class="rt-setting-input rt-setting-input-short" type="number" min="5" aria-label="刷新间隔（秒）" @change="emitAutoRefreshSettings" />
      <span class="muted rt-setting-note">秒</span>
    </SettingRow>
  </SettingSection>
</template>
