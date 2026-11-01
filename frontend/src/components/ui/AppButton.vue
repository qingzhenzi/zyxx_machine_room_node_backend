<script setup lang="ts">
import { computed } from 'vue'

import AppIcon from './AppIcon.vue'

type Variant = 'primary' | 'secondary' | 'ghost' | 'danger'
type Size = 'sm' | 'md'

const props = withDefaults(
  defineProps<{
    variant?: Variant
    size?: Size
    icon?: string
    iconRight?: string
    loading?: boolean
    disabled?: boolean
    type?: 'button' | 'submit' | 'reset'
  }>(),
  {
    variant: 'secondary',
    size: 'md',
    icon: '',
    iconRight: '',
    loading: false,
    disabled: false,
    type: 'button',
  },
)

const variantClass = computed(
  () =>
    ({
      primary: 'bg-accent text-white hover:bg-accent-strong',
      secondary:
        'border border-line bg-surface text-ink-soft hover:border-line-strong hover:text-ink',
      ghost: 'text-muted hover:bg-canvas hover:text-ink',
      danger:
        'border border-line bg-surface text-ink-soft hover:border-rose-200 hover:bg-rose-50 hover:text-rose-600',
    })[props.variant],
)

const sizeClass = computed(() =>
  props.size === 'sm' ? 'h-8 px-3 text-[12px]' : 'h-9 px-3.5 text-[12.5px]',
)

const iconSize = computed(() => (props.size === 'sm' ? 12 : 13))
</script>

<template>
  <button
    :type="type"
    :disabled="disabled || loading"
    class="inline-flex shrink-0 items-center justify-center gap-1.5 rounded-md font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/30 disabled:cursor-not-allowed disabled:opacity-50"
    :class="[variantClass, sizeClass]"
  >
    <AppIcon v-if="loading" name="refresh" :size="iconSize" class="animate-spin" />
    <AppIcon v-else-if="icon" :name="icon" :size="iconSize" />
    <slot />
    <AppIcon v-if="iconRight" :name="iconRight" :size="iconSize" />
  </button>
</template>
