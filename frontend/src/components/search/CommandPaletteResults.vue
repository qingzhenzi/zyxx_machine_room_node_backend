<script setup lang="ts">
import AppIcon from '@/components/ui/AppIcon.vue'

import type { PaletteEntry } from './types'

defineProps<{
  entries: PaletteEntry[]
  activeIndex: number
  loading: boolean
  query: string
}>()

const emit = defineEmits<{
  hover: [index: number]
  select: [entry: PaletteEntry]
}>()
</script>

<template>
  <div class="max-h-[420px] overflow-y-auto p-2">
    <p v-if="query.trim() && loading" class="px-3 py-8 text-center text-[12px] text-muted">
      搜索中…
    </p>

    <template v-else-if="entries.length">
      <p class="px-2.5 pb-1 pt-1.5 text-[10px] font-medium tracking-[0.06em] text-faint">
        {{ query.trim() ? '搜索结果' : '快捷操作' }}
      </p>
      <button
        v-for="(entry, index) in entries"
        :key="entry.key"
        type="button"
        class="flex w-full items-center gap-3 rounded-md px-2.5 py-2 text-left transition-colors"
        :class="index === activeIndex ? 'bg-accent-soft' : 'hover:bg-canvas'"
        @mouseenter="emit('hover', index)"
        @click="emit('select', entry)"
      >
        <span
          class="flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-line bg-surface"
          :class="index === activeIndex ? 'text-accent' : 'text-muted'"
        >
          <AppIcon :name="entry.icon" :size="15" />
        </span>
        <span class="min-w-0 flex-1">
          <span class="block truncate text-[12.5px] font-medium text-ink">{{ entry.title }}</span>
          <span class="mt-0.5 block truncate font-mono text-[10.5px] text-faint">
            {{ entry.subtitle }}
          </span>
        </span>
        <span class="shrink-0 text-[10px] text-faint">{{ entry.section }}</span>
        <AppIcon name="chevron-right" :size="13" class="shrink-0 text-faint" />
      </button>
    </template>

    <p v-else-if="query.trim()" class="px-3 py-8 text-center text-[12px] text-muted">
      没有找到「{{ query }}」相关结果
    </p>

    <p v-else class="px-3 py-8 text-center text-[12px] text-muted">输入关键词开始搜索</p>
  </div>
</template>
