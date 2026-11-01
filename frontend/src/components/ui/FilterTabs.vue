<script setup lang="ts">
interface FilterTab {
  value: string
  label: string
  count?: number
}

defineProps<{
  modelValue: string
  tabs: FilterTab[]
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  change: [value: string]
}>()

function select(value: string) {
  emit('update:modelValue', value)
  emit('change', value)
}
</script>

<template>
  <div class="flex items-center gap-0.5 rounded-md border border-line bg-surface p-0.5">
    <button
      v-for="tab in tabs"
      :key="tab.value"
      type="button"
      class="flex items-center gap-1.5 rounded px-2 py-1 text-[11.5px] transition-colors"
      :class="
        modelValue === tab.value ? 'bg-canvas font-medium text-ink' : 'text-muted hover:text-ink'
      "
      @click="select(tab.value)"
    >
      {{ tab.label }}
      <span v-if="tab.count !== undefined" class="font-mono text-[9.5px] text-faint">
        {{ tab.count }}
      </span>
    </button>
  </div>
</template>
