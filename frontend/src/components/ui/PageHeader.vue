<script setup lang="ts">
withDefaults(
  defineProps<{
    title: string
    description?: string
    badge?: string
  }>(),
  { description: '', badge: '' },
)
</script>

<template>
  <div class="flex items-end justify-between gap-4">
    <div class="min-w-0">
      <div class="flex items-center gap-2.5">
        <h1 class="text-[22px] font-semibold tracking-[-0.02em] text-ink">{{ title }}</h1>
        <span
          v-if="badge"
          class="rounded-full border border-line bg-surface px-2 py-0.5 text-[10.5px] font-medium text-muted"
        >
          {{ badge }}
        </span>
      </div>
      <p v-if="description || $slots.description" class="mt-1 text-[12px] text-muted">
        <slot name="description">{{ description }}</slot>
      </p>
    </div>

    <div v-if="$slots.actions" class="flex shrink-0 items-center gap-2">
      <slot name="actions" />
    </div>
  </div>
</template>
