<script setup>
import { ICONS } from './icons'

defineProps({
  name: { type: String, required: true },
  size: { type: Number, default: 20 }
})
</script>

<template>
  <svg
    class="rt-icon"
    viewBox="0 0 24 24"
    :width="size"
    :height="size"
    fill="none"
    stroke="currentColor"
    stroke-width="1.75"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    v-html="ICONS[name]"
  />
</template>
