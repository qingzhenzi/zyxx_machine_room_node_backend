<script setup lang="ts">
import { ref } from 'vue'

import AppIcon from './AppIcon.vue'
import { useToastStore } from '@/stores/toast'

const props = withDefaults(
  defineProps<{
    value: string | null | undefined
    label?: string
    display?: string
    mono?: boolean
    truncate?: boolean
  }>(),
  { label: '', display: '', mono: true, truncate: false },
)

const toast = useToastStore()
const copied = ref(false)

async function copy() {
  if (!props.value) return
  try {
    await navigator.clipboard.writeText(props.value)
    copied.value = true
    toast.success(props.label ? `${props.label}已复制` : '已复制到剪贴板')
    setTimeout(() => {
      copied.value = false
    }, 1_500)
  } catch {
    toast.error('复制失败', '浏览器未授予剪贴板权限')
  }
}
</script>

<template>
  <span
    class="group inline-flex min-w-0 items-center gap-1.5"
    :class="truncate ? 'max-w-full' : ''"
  >
    <span
      :class="[
        mono ? 'font-mono text-[11.5px]' : 'text-[11.5px]',
        truncate ? 'truncate' : 'break-all',
        'text-ink-soft',
      ]"
    >
      {{ display || value || '—' }}
    </span>
    <button
      v-if="value"
      type="button"
      :aria-label="label ? `复制${label}` : '复制'"
      class="shrink-0 rounded p-0.5 text-faint opacity-0 transition-opacity hover:text-ink group-hover:opacity-100 focus-visible:opacity-100"
      @click="copy"
    >
      <AppIcon :name="copied ? 'check' : 'copy'" :size="12" />
    </button>
  </span>
</template>
