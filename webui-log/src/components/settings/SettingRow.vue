<script setup>
// One settings line: name on the left, a plain explanation in the middle, the current
// value and its actions on the right (stacked on narrow screens).
defineProps({
  label: { type: String, required: true },
  description: { type: String, default: '' }
})
</script>

<template>
  <div class="rt-setting-row">
    <div class="rt-setting-label">{{ label }}</div>
    <div class="rt-setting-desc">
      <slot name="description">{{ description }}</slot>
    </div>
    <div class="rt-setting-value">
      <slot />
    </div>
  </div>
</template>
