<script setup>
import { ref, watch } from 'vue'
import { DURATION_UNITS, splitDuration, toSeconds } from '../../utils/tuning'

// A duration typed as an amount and a unit; the model is whole seconds (NaN while the
// amount is not a number, which the range check of the page refuses).
const props = defineProps({
  modelValue: { type: Number, default: 0 },
  label: { type: String, required: true },
  disabled: { type: Boolean, default: false }
})
const emit = defineEmits(['update:modelValue'])

const start = splitDuration(props.modelValue)
const amount = ref(start.value)
const unit = ref(start.unit)

// Follow the model when it changes from outside (a reload, "恢复默认"), not while it is
// what this input just produced.
watch(() => props.modelValue, (seconds) => {
  if (toSeconds(amount.value, unit.value) === seconds) return
  const next = splitDuration(seconds)
  amount.value = next.value
  unit.value = next.unit
})

function update() {
  emit('update:modelValue', toSeconds(amount.value, unit.value))
}
</script>

<template>
  <span class="rt-duration">
    <input v-model="amount" class="rt-setting-input rt-setting-input-short" type="number" min="0" step="any" :disabled="disabled" :aria-label="label" @input="update" />
    <select v-model="unit" class="rt-setting-input rt-setting-input-short" :disabled="disabled" :aria-label="`${label}的单位`" @change="update">
      <option v-for="item in DURATION_UNITS" :key="item.id" :value="item.id">{{ item.label }}</option>
    </select>
  </span>
</template>
