<script setup>
import { useRouter } from 'vue-router'
import QueryManager from '../components/QueryManager.vue'

// Diagnostic capture is rarely used: a page below 日志, reached from the live query page.
const router = useRouter()
</script>

<template>
  <QueryManager mode="diagnostic" data-diagnostic-page @close="router.push('/logs')" />
</template>
