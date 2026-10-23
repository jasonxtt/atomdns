<script setup>
defineProps({
  id: { type: String, required: true },
  title: { type: String, required: true }
})
</script>

<template>
  <section :id="`system-${id}`" class="panel rt-setting-section" :data-section="id">
    <h3 class="rt-setting-section-title">{{ title }}</h3>
    <slot />
  </section>
</template>
