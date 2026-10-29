<script setup>
// One settings line: name on the left, a plain explanation in the middle, the current
// value and its actions on the right (stacked on narrow screens). Inside a tile grid
// `layout` picks where the value goes: "inline" beside the name (a switch), "stat" under
// the name (a read-only figure) or "form" under the explanation (buttons and fields).
defineProps({
  label: { type: String, required: true },
  description: { type: String, default: '' },
  layout: { type: String, default: 'inline' }
})
</script>

<template>
  <div class="rt-setting-row" :class="layout !== 'inline' && `rt-setting-row--${layout}`">
    <div class="rt-setting-label">{{ label }}</div>
    <div class="rt-setting-desc">
      <slot name="description">{{ description }}</slot>
    </div>
    <div class="rt-setting-value">
      <slot />
    </div>
  </div>
</template>
