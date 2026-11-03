<script setup>
import { computed } from 'vue'

// A small trend line with a soft fill under it. `values` are plain numbers, oldest first.
const props = defineProps({
  values: { type: Array, default: () => [] },
  color: { type: String, default: 'var(--rp-pine)' }
})

const W = 100
const H = 32
const points = computed(() => {
  const values = props.values
  if (values.length < 2) return []
  const min = Math.min(...values)
  const max = Math.max(...values)
  // A tiny wobble must not fill the box: the line spans at least 5% of the value, centred.
  const span = Math.max(max - min, Math.abs(max) * 0.05, 1e-6)
  const base = (min + max) / 2 - span / 2
  return values.map((value, index) => [(index / (values.length - 1)) * W, H - 3 - ((value - base) / span) * (H - 6)])
})
const line = computed(() => points.value.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(' '))
const area = computed(() => (points.value.length ? `0,${H} ${line.value} ${W},${H}` : ''))
</script>

<template>
  <svg class="rt-spark" :style="{ '--spark': color }" :viewBox="`0 0 ${W} ${H}`" preserveAspectRatio="none" aria-hidden="true">
    <template v-if="points.length">
      <polygon class="rt-spark-area" :points="area" />
      <polyline class="rt-spark-line" :points="line" />
    </template>
    <line v-else class="rt-spark-line" x1="0" y1="29" x2="100" y2="29" />
  </svg>
</template>

<style scoped>
.rt-spark {
  display: block;
  width: 100%;
  height: 100%;
}

.rt-spark-line {
  fill: none;
  stroke: var(--spark);
  stroke-width: 1.6;
  stroke-linejoin: round;
  stroke-linecap: round;
  vector-effect: non-scaling-stroke;
}

.rt-spark-area {
  fill: color-mix(in srgb, var(--spark) 16%, transparent);
}
</style>
