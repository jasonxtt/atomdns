<script setup>
// The brand mark: a rounded square in the accent colour holding an "r" whose arm ends in a
// gold dot, like a query leaving a resolver. The favicon in index.html draws the same shape.
defineProps({ size: { type: Number, default: 28 } })
</script>

<template>
  <svg class="rt-brand-mark" viewBox="0 0 32 32" :width="size" :height="size" aria-hidden="true" focusable="false">
    <rect width="32" height="32" rx="9" fill="var(--rp-pine)" />
    <path d="M11 23V10.5" fill="none" stroke="#fffaf3" stroke-width="3" stroke-linecap="round" />
    <path d="M11 16c0-3.3 2.4-5.5 6-5.5" fill="none" stroke="#fffaf3" stroke-width="3" stroke-linecap="round" />
    <circle cx="23" cy="10.5" r="2.6" fill="var(--rp-gold)" />
  </svg>
</template>
