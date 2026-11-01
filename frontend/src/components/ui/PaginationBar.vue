<script setup lang="ts">
import AppButton from './AppButton.vue'

withDefaults(
  defineProps<{
    page: number
    totalPages: number
    total: number
    label?: string
  }>(),
  { label: '条记录' },
)

const emit = defineEmits<{
  change: [page: number]
}>()
</script>

<template>
  <div
    class="flex items-center justify-between border-t border-line bg-canvas/40 px-5 py-2.5 text-[11px] text-muted"
  >
    <span>第 {{ page }} / {{ totalPages }} 页 · 共 {{ total }} {{ label }}</span>
    <div class="flex items-center gap-1.5">
      <AppButton size="sm" :disabled="page <= 1" @click="emit('change', page - 1)">
        上一页
      </AppButton>
      <AppButton size="sm" :disabled="page >= totalPages" @click="emit('change', page + 1)">
        下一页
      </AppButton>
    </div>
  </div>
</template>
