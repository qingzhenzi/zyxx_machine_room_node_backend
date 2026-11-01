<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue'
import { useToastStore } from '@/stores/toast'

const toast = useToastStore()

const tone: Record<string, { icon: string; box: string }> = {
  success: { icon: 'check', box: 'bg-emerald-50 text-emerald-600' },
  error: { icon: 'alert', box: 'bg-rose-50 text-rose-600' },
  info: { icon: 'info', box: 'bg-sky-50 text-sky-600' },
}
</script>

<template>
  <div
    class="pointer-events-none fixed bottom-9 right-5 z-[70] flex w-[330px] flex-col gap-2"
    aria-live="polite"
  >
    <div
      v-for="item in toast.items"
      :key="item.id"
      class="pointer-events-auto flex items-start gap-2.5 rounded-lg border border-line bg-surface p-3 shadow-[0_12px_30px_-12px_rgba(16,24,40,0.35)]"
    >
      <span
        class="mt-0.5 flex h-5 w-5 shrink-0 items-center justify-center rounded-full"
        :class="(tone[item.kind] ?? tone.info)?.box"
      >
        <AppIcon :name="(tone[item.kind] ?? tone.info)?.icon ?? 'info'" :size="12" />
      </span>
      <div class="min-w-0 flex-1">
        <p class="text-[12px] font-medium text-ink">{{ item.title }}</p>
        <p v-if="item.description" class="mt-0.5 text-[10.5px] leading-4 text-muted">
          {{ item.description }}
        </p>
      </div>
      <button
        type="button"
        aria-label="关闭提示"
        class="rounded p-0.5 text-faint transition-colors hover:text-ink"
        @click="toast.dismiss(item.id)"
      >
        <AppIcon name="x" :size="13" />
      </button>
    </div>
  </div>
</template>
