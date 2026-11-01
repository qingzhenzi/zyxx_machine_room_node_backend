<script setup lang="ts">
defineProps<{
  items: { label: string; value?: string | number | null; mono?: boolean }[]
  columns?: 1 | 2
}>()
</script>

<template>
  <dl class="grid gap-x-6 gap-y-3" :class="columns === 2 ? 'grid-cols-2' : 'grid-cols-1'">
    <div v-for="item in items" :key="item.label" class="min-w-0">
      <dt class="text-[10.5px] text-faint">{{ item.label }}</dt>
      <dd
        class="mt-0.5 break-words text-[12px] text-ink"
        :class="item.mono ? 'font-mono text-[11.5px]' : ''"
      >
        <slot :name="item.label" :item="item">{{ item.value ?? '—' }}</slot>
      </dd>
    </div>
  </dl>
</template>
