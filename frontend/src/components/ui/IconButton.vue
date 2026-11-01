<script setup lang="ts">
import { computed } from 'vue'

import AppIcon from './AppIcon.vue'

const props = withDefaults(
  defineProps<{
    icon: string
    label: string
    size?: 'sm' | 'md'
    variant?: 'ghost' | 'outline'
    loading?: boolean
    disabled?: boolean
  }>(),
  {
    size: 'md',
    variant: 'ghost',
    loading: false,
    disabled: false,
  },
)

const sizeClass = computed(() => (props.size === 'sm' ? 'h-7 w-7' : 'h-9 w-9'))

const variantClass = computed(() =>
  props.variant === 'outline'
    ? 'border border-line bg-surface text-muted hover:border-line-strong hover:text-ink'
    : 'text-muted hover:bg-canvas hover:text-ink',
)

const iconSize = computed(() => (props.size === 'sm' ? 13 : 14))
</script>

<template>
  <button
    type="button"
    :aria-label="label"
    :title="label"
    :disabled="disabled || loading"
    class="flex shrink-0 items-center justify-center rounded-md transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/30 disabled:cursor-not-allowed disabled:opacity-50"
    :class="[sizeClass, variantClass]"
  >
    <AppIcon
      :name="loading ? 'refresh' : icon"
      :size="iconSize"
      :class="loading ? 'animate-spin' : ''"
    />
  </button>
</template>
