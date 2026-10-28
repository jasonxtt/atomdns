<script setup>
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
import OverridesPanel from '../../components/rules/OverridesPanel.vue'
import UpstreamPanel from '../../components/rules/UpstreamPanel.vue'
</script>

<template>
  <div class="rt-settings-page">
    <FlagSwitches page="upstream" />
    <UpstreamPanel />
    <OverridesPanel />
  </div>
</template>
