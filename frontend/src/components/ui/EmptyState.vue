<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue'

withDefaults(
  defineProps<{
    title: string
    description?: string
    icon?: string
  }>(),
  { description: '', icon: 'spark' },
)
</script>

<template>
  <div class="flex flex-col items-center justify-center px-6 py-16 text-center">
    <div
      class="flex h-11 w-11 items-center justify-center rounded-full border border-line bg-canvas text-faint"
    >
      <AppIcon :name="icon" :size="19" />
    </div>
    <p class="mt-4 text-[13px] font-semibold text-ink">{{ title }}</p>
    <p v-if="description" class="mt-1.5 max-w-[380px] text-[11.5px] leading-5 text-muted">
      {{ description }}
    </p>
    <div v-if="$slots.action" class="mt-4">
      <slot name="action" />
    </div>
  </div>
</template>
