<script setup lang="ts">
/** TOP 排行（房间 / 设备）：名次 + 主标题 + 副标题 + 数量。 */
import { maxCount } from './ticketStats'

interface RankItem {
  name: string
  sub?: string | null
  count: number
}

const props = withDefaults(
  defineProps<{
    items: RankItem[]
    emptyText?: string
    loading?: boolean
  }>(),
  { emptyText: '所选区间暂无数据', loading: false },
)
</script>

<template>
  <div class="px-5 py-4">
    <div v-if="props.loading" class="space-y-3">
      <div v-for="row in 5" :key="row" class="h-5 animate-pulse rounded bg-canvas" />
    </div>
    <p v-else-if="!props.items.length" class="py-8 text-center text-[11.5px] text-faint">
      {{ props.emptyText }}
    </p>
    <div v-else class="flex flex-col gap-2.5">
      <div
        v-for="(item, index) in props.items"
        :key="`${item.name}-${index}`"
        class="flex items-center gap-2.5"
      >
        <span
          class="flex h-5 w-5 shrink-0 items-center justify-center rounded-md font-mono text-[10px]"
          :class="index === 0 ? 'bg-accent-soft text-accent' : 'bg-canvas text-faint'"
        >
          {{ index + 1 }}
        </span>
        <span class="min-w-0 flex-1">
          <span class="block truncate text-[11.5px] text-ink" :title="item.name">{{
            item.name
          }}</span>
          <span v-if="item.sub" class="mt-0.5 block truncate text-[10px] text-faint">{{
            item.sub
          }}</span>
        </span>
        <span class="shrink-0 font-mono text-[11px] tabular-nums text-ink-soft">
          {{ item.count }}<small class="ml-0.5 text-[9.5px] text-faint">张</small>
        </span>
      </div>
    </div>
    <div
      v-if="!props.loading && props.items.length"
      class="mt-3 flex items-center justify-between border-t border-line/70 pt-3 text-[10.5px] text-faint"
    >
      <span>TOP {{ props.items.length }}</span>
      <span>最高 {{ maxCount(props.items) }} 张</span>
    </div>
  </div>
</template>
