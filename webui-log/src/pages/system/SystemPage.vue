<script setup>
import SystemAuditSection from './SystemAuditSection.vue'
import SystemServiceSection from './SystemServiceSection.vue'
import SystemUiSection from './SystemUiSection.vue'
import SystemUpdateSection from './SystemUpdateSection.vue'
</script>

<template>
  <div class="rt-settings-page">
    <SystemUpdateSection />
    <SystemAuditSection />
    <SystemUiSection />
    <SystemServiceSection />
  </div>
</template>
