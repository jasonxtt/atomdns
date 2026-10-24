<script setup>
import DomainGenerationPanel from '../../components/rules/DomainGenerationPanel.vue'
import FlagSwitches from '../../components/rules/FlagSwitches.vue'
import LearningData from '../../components/rules/LearningData.vue'
import RequeryPanel from '../../components/rules/RequeryPanel.vue'
</script>

<template>
  <div class="rt-settings-page">
    <FlagSwitches page="learning" />
    <DomainGenerationPanel />
    <LearningData />
    <RequeryPanel />
  </div>
</template>
