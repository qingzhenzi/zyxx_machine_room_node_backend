<script setup lang="ts">
import AppIcon from './AppIcon.vue'

withDefaults(
  defineProps<{
    placeholder?: string
    label?: string
  }>(),
  { placeholder: '搜索', label: '搜索' },
)

const model = defineModel<string>({ required: true })
</script>

<template>
  <label
    class="relative flex h-9 items-center gap-2 rounded-md border border-line bg-surface px-3 focus-within:border-accent-line"
  >
    <AppIcon name="search" :size="14" class="text-faint" />
    <input
      v-model="model"
      type="search"
      :aria-label="label"
      :placeholder="placeholder"
      class="h-full w-full min-w-0 bg-transparent text-[12px] text-ink placeholder:text-faint focus:outline-none"
    />
  </label>
</template>
