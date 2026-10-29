<script setup>
import CachePanel from '../../components/rules/CachePanel.vue'
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
</script>

<template>
  <div class="rt-settings-page">
    <FlagSwitches page="cache" tiles />
    <CachePanel />
    <p class="muted rt-page-note">清缓存后的刷新与预热在 <router-link to="/rules/learning">域名学习 › 刷新缓存与预热</router-link>。</p>
  </div>
</template>
