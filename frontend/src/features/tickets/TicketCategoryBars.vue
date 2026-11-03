<script setup lang="ts">
/** 分类分布横向条形图（纯 CSS）。 */
import { computed } from 'vue'

import { maxCount, percentage, type CategoryCountView } from './ticketStats'
import { ticketCategoryMeta } from './ticketMeta'

const props = withDefaults(
  defineProps<{
    items: CategoryCountView[]
    loading?: boolean
  }>(),
  { loading: false },
)

const total = computed(() => props.items.reduce((sum, item) => sum + item.count, 0))
const max = computed(() => maxCount(props.items))
</script>

<template>
  <div class="px-5 py-4">
    <div v-if="props.loading" class="space-y-3">
      <div v-for="row in 5" :key="row" class="h-5 animate-pulse rounded bg-canvas" />
    </div>

    <p v-else-if="!props.items.length" class="py-8 text-center text-[11.5px] text-faint">
      所选区间暂无工单数据。
    </p>

    <div v-else class="flex flex-col gap-3">
      <div v-for="item in props.items" :key="item.category" class="flex items-center gap-2.5">
        <span
          class="w-[74px] shrink-0 truncate text-[11px] text-ink-soft"
          :title="ticketCategoryMeta(item.category).label"
        >
          {{ ticketCategoryMeta(item.category).label }}
        </span>
        <span class="h-2 min-w-0 flex-1 overflow-hidden rounded-full bg-canvas">
          <span
            class="block h-full rounded-full transition-[width] duration-500"
            :class="ticketCategoryMeta(item.category).barClass"
            :style="{ width: `${Math.max(2, (item.count / max) * 100)}%` }"
          />
        </span>
        <span class="w-9 shrink-0 text-right font-mono text-[10.5px] tabular-nums text-muted">
          {{ item.count }}
        </span>
      </div>
    </div>

    <div
      v-if="!props.loading && props.items.length"
      class="mt-4 flex items-center justify-between border-t border-line/70 pt-3 text-[10.5px] text-faint"
    >
      <span
        >共 <b class="text-ink">{{ total }}</b> 张工单</span
      >
      <span>第一类占 {{ percentage(props.items[0]?.count ?? 0, total) }}%</span>
    </div>
  </div>
</template>
