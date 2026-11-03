<script setup lang="ts">
/** 处理人负载：未关闭（active）与已解决（resolved）堆叠条 + 平均处理时长。 */
import { computed } from 'vue'

import { formatMttr, type AssigneeLoadView } from './ticketStats'

const props = withDefaults(
  defineProps<{
    items: AssigneeLoadView[]
    loading?: boolean
  }>(),
  { loading: false },
)

const maxTotal = computed(() => {
  let max = 1
  for (const item of props.items) max = Math.max(max, item.active + item.resolved)
  return max
})

function total(item: AssigneeLoadView): number {
  return item.active + item.resolved
}

function nameOf(item: AssigneeLoadView): string {
  return item.display_name || '未分配'
}

function initial(item: AssigneeLoadView): string {
  return nameOf(item).slice(0, 1)
}
</script>

<template>
  <div class="px-5 py-4">
    <div v-if="props.loading" class="space-y-3">
      <div v-for="row in 4" :key="row" class="h-5 animate-pulse rounded bg-canvas" />
    </div>

    <p v-else-if="!props.items.length" class="py-8 text-center text-[11.5px] text-faint">
      所选区间没有处理人负载数据。
    </p>

    <template v-else>
      <div class="flex flex-col gap-3">
        <div
          v-for="item in props.items"
          :key="item.assignee_id ?? nameOf(item)"
          class="flex items-center gap-2.5"
        >
          <span
            class="flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-canvas text-[10px] font-medium text-muted"
          >
            {{ initial(item) }}
          </span>
          <span class="min-w-0 flex-1">
            <span class="flex items-center justify-between gap-2">
              <span class="truncate text-[11.5px] text-ink">{{ nameOf(item) }}</span>
              <span class="shrink-0 font-mono text-[10.5px] tabular-nums text-faint">
                {{ total(item) }}
              </span>
            </span>
            <span class="mt-1 flex h-1.5 overflow-hidden rounded-full bg-canvas">
              <span
                class="h-full bg-amber-500"
                :style="{ width: `${(item.active / maxTotal) * 100}%` }"
                :title="`未关闭 ${item.active} 张`"
              />
              <span
                class="h-full bg-emerald-500"
                :style="{ width: `${(item.resolved / maxTotal) * 100}%` }"
                :title="`已解决 ${item.resolved} 张`"
              />
            </span>
            <span class="mt-1 block text-[9.5px] text-faint">
              未关闭 {{ item.active }} · 已解决 {{ item.resolved }} · 平均
              {{ formatMttr(item.avg_resolve_secs) }}
            </span>
          </span>
        </div>
      </div>

      <div class="mt-3 flex items-center gap-4 border-t border-line/70 pt-3 text-[10px] text-muted">
        <span class="flex items-center gap-1.5">
          <i class="h-2 w-2 rounded-sm bg-amber-500" aria-hidden="true" />未关闭
        </span>
        <span class="flex items-center gap-1.5">
          <i class="h-2 w-2 rounded-sm bg-emerald-500" aria-hidden="true" />已解决
        </span>
      </div>
    </template>
  </div>
</template>
