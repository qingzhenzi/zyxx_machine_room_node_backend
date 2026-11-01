<script setup lang="ts">
import type { Transfer } from '@/api/schema'
import EmptyState from '@/components/ui/EmptyState.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'
import { formatRelative } from '@/utils/format'

defineProps<{
  transfers: Transfer[]
}>()

const statusMeta: Record<string, { label: string; tone: BadgeTone }> = {
  pending: { label: '等待中', tone: 'slate' },
  running: { label: '进行中', tone: 'amber' },
  completed: { label: '已完成', tone: 'emerald' },
  failed: { label: '失败', tone: 'rose' },
  expired: { label: '已过期', tone: 'slate' },
}

function meta(status: string | undefined) {
  return statusMeta[status ?? ''] ?? { label: status ?? '—', tone: 'slate' as const }
}
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">待处理分发任务</h2>
        <p class="mt-0.5 text-[11px] text-muted">镜像分发 / 对象收集的进行中与失败任务</p>
      </div>
      <RouterLink :to="{ name: 'storage' }" class="text-[11px] text-accent hover:underline">
        前往存储
      </RouterLink>
    </header>

    <ul v-if="transfers.length" class="divide-y divide-line/70">
      <li
        v-for="transfer in transfers"
        :key="transfer.id"
        class="flex items-center gap-3 px-5 py-2.5"
      >
        <StateBadge :label="meta(transfer.status).label" :tone="meta(transfer.status).tone" />
        <span class="min-w-0 flex-1 truncate font-mono text-[10.5px] text-ink-soft">
          {{ transfer.room_id?.slice(0, 8) }} · {{ transfer.image_id?.slice(0, 8) ?? '对象' }}
        </span>
        <span class="shrink-0 font-mono text-[10.5px] text-faint"
          >{{ transfer.progress ?? 0 }}%</span
        >
        <span class="shrink-0 text-[10px] text-faint">
          {{ formatRelative(transfer.updated_at ?? transfer.created_at) }}
        </span>
      </li>
    </ul>

    <EmptyState
      v-else
      title="没有待处理任务"
      description="镜像分发与对象收集全部完成时，这里会保持为空。"
      icon="database"
    />
  </section>
</template>
