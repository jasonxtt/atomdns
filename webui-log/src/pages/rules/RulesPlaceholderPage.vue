<script setup>
defineProps({
  title: { type: String, required: true },
  description: { type: String, default: '' }
})
</script>

<template>
  <section class="panel rt-placeholder" data-placeholder>
    <h3>{{ title }}</h3>
    <p class="muted">本项将在 UI 重做第二段实现。</p>
    <p v-if="description" class="muted">{{ description }}</p>
  </section>
</template>
