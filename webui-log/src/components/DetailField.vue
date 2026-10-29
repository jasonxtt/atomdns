<script setup>
import { computed } from 'vue'
import AppIcon from './AppIcon.vue'

// One field of a detail dialog: a small label over its value. A field that can be copied or
// used as a log filter takes the `{ value, copyValue, filterValue, exact, mono }` object
// instead of a plain `value`, and shows the two icon buttons beside the value.
const props = defineProps({
  label: { type: String, required: true },
  value: { type: String, default: '' },
  field: { type: Object, default: null },
  wide: { type: Boolean, default: false }
})
defineEmits(['copy', 'filter'])

const shown = computed(() => (props.field ? props.field.value : props.value) || '-')
</script>

<template>
  <div class="detail-field" :class="{ 'detail-wide': wide }">
    <span class="detail-label">{{ label }}</span>
    <span class="detail-value">
      <span class="detail-text" :class="{ mono: field?.mono }">{{ shown }}</span>
      <span v-if="field" class="detail-inline-actions">
        <button class="rt-row-icon" type="button" title="复制" :aria-label="`复制${label}`" :disabled="!field.copyValue" @click="$emit('copy', field.copyValue)">
          <AppIcon name="copy" :size="15" />
        </button>
        <button class="rt-row-icon" type="button" title="按此筛选" :aria-label="`按${label}筛选`" :disabled="!field.filterValue" @click="$emit('filter', field.filterValue, field.exact)">
          <AppIcon name="filter" :size="15" />
        </button>
      </span>
    </span>
  </div>
</template>
