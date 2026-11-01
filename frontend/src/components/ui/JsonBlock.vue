<script setup lang="ts">
import { computed, ref } from 'vue'

import AppIcon from './AppIcon.vue'

const props = withDefaults(
  defineProps<{
    value: unknown
    label?: string
    collapsible?: boolean
    maxHeight?: string
  }>(),
  { label: '', collapsible: false, maxHeight: '360px' },
)

const open = ref(!props.collapsible)
const text = computed(() => JSON.stringify(props.value ?? null, null, 2) ?? 'null')
</script>

<template>
  <div class="overflow-hidden rounded-md border border-line bg-canvas/50">
    <button
      v-if="collapsible"
      type="button"
      class="flex w-full items-center justify-between px-3 py-2 text-left text-[11px] text-muted transition-colors hover:text-ink"
      @click="open = !open"
    >
      <span>{{ label || 'JSON' }}</span>
      <AppIcon :name="open ? 'chevron-down' : 'chevron-right'" :size="13" />
    </button>
    <pre
      v-if="open"
      class="overflow-auto px-3 py-2.5 font-mono text-[10.5px] leading-[1.7] text-ink-soft"
      :style="{ maxHeight }"
      >{{ text }}</pre>
  </div>
</template>
