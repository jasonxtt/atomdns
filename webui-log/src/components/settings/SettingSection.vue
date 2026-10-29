<script setup>
// `tiles` puts the section's rows side by side in one box, a vertical line between them,
// instead of one row per line inside a card: the rows of a group belong together.
defineProps({
  id: { type: String, required: true },
  title: { type: String, required: true },
  tiles: { type: Boolean, default: false }
})
</script>

<template>
  <section :id="`system-${id}`" class="panel rt-setting-section" :class="{ 'rt-setting-tiles rt-setting-tiles-compact rt-setting-joined': tiles }" :data-section="id">
    <h3 class="rt-setting-section-title">{{ title }}</h3>
    <div v-if="tiles" class="rt-setting-grid">
      <slot />
    </div>
    <slot v-else />
  </section>
</template>
