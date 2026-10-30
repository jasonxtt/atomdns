<script setup>
// The brand mark: the square AtomDNS logo (docs/assets/atomdns-icon.png is the same picture).
import markUrl from '../assets/atomdns-mark.png'

defineProps({ size: { type: Number, default: 32 } })
</script>

<template>
  <img class="rt-brand-mark" :src="markUrl" :width="size" :height="size" alt="" aria-hidden="true" />
</template>
