<script setup>
import { useRouter } from 'vue-router'
import QueryManager from '../components/QueryManager.vue'

const router = useRouter()
</script>

<template>
  <QueryManager mode="live" @open-diagnostic="router.push('/logs/diagnostic')" />
</template>
