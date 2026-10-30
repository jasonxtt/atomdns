<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getJSON } from '../../api/http'
import { supportsOperation } from '../../api/runtimeCapabilities'

// A read-only reference to the special groups with a link to each (the lists and
// subscriptions of a group are managed in the group's own page).
defineProps({
  title: { type: String, default: '专属分流组的名单' },
  note: { type: String, default: '专属分流组的手动名单和订阅在组里管理，这里只列出来。' }
})

const groups = ref([])
const allowed = computed(() => supportsOperation('groups.read'))

async function load() {
  if (!allowed.value) return
  try {
    const data = await getJSON('/api/v1/special-groups')
    groups.value = Array.isArray(data) ? data : []
  } catch {
    groups.value = []
  }
}

onMounted(() => {
  load()
  window.addEventListener('atomdns-log-refresh', load)
})
onBeforeUnmount(() => window.removeEventListener('atomdns-log-refresh', load))
</script>

<template>
  <section v-if="allowed && groups.length > 0" class="panel rt-group-links" data-special-links>
    <h3>{{ title }}</h3>
    <p class="muted rt-list-hint">{{ note }}</p>
    <ul class="rt-link-list">
      <li v-for="group in groups" :key="group.slot">
        <span>{{ group.name }}<span v-if="group.listen_port" class="muted"> · 端口 {{ group.listen_port }}</span></span>
        <router-link :to="{ path: '/rules/special-groups', query: { group: String(group.slot) } }" class="btn tiny secondary">管理</router-link>
      </li>
    </ul>
  </section>
</template>
