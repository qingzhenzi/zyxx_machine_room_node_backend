<script setup lang="ts">
import type { MachineSnapshotPoint } from '@/api/schema'
import EmptyState from '@/components/ui/EmptyState.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { machineHealthMeta } from './machineMeta'
import { formatDateTime, formatRelative } from '@/utils/format'

defineProps<{
  points: MachineSnapshotPoint[]
}>()

const emit = defineEmits<{
  select: [point: MachineSnapshotPoint]
}>()
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">历史快照</h2>
        <p class="mt-0.5 text-[11px] text-muted">
          按采集时间倒序，保留策略由服务端 MACHINE_SNAPSHOT_KEEP 控制
        </p>
      </div>
      <span class="text-[11px] text-faint">{{ points.length }} 条</span>
    </header>

    <div v-if="points.length" class="overflow-x-auto">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">采集时间</th>
            <th class="table-th">健康</th>
            <th class="table-th text-right">分数</th>
            <th class="table-th text-right">CPU</th>
            <th class="table-th text-right">内存</th>
            <th class="table-th text-right">磁盘</th>
            <th class="table-th">Agent</th>
            <th class="table-th pr-5 text-right">接收时间</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="point in points"
            :key="point.id"
            class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            @click="emit('select', point)"
          >
            <td class="table-td pl-5 text-[11.5px]">
              <span class="text-ink-soft">{{ formatDateTime(point.collected_at) }}</span>
              <span class="ml-1 text-[10px] text-faint">{{
                formatRelative(point.collected_at)
              }}</span>
            </td>
            <td class="table-td">
              <StateBadge
                :label="machineHealthMeta(point.health_status).label"
                :tone="machineHealthMeta(point.health_status).tone"
              />
            </td>
            <td class="table-td text-right font-mono text-[11px] tabular-nums text-ink-soft">
              {{ point.health_score ?? '—' }}
            </td>
            <td class="table-td text-right font-mono text-[11px] tabular-nums text-muted">
              {{
                point.cpu_usage_percent === null || point.cpu_usage_percent === undefined
                  ? '—'
                  : `${point.cpu_usage_percent.toFixed(0)}%`
              }}
            </td>
            <td class="table-td text-right font-mono text-[11px] tabular-nums text-muted">
              {{
                point.memory_usage_percent === null || point.memory_usage_percent === undefined
                  ? '—'
                  : `${point.memory_usage_percent.toFixed(0)}%`
              }}
            </td>
            <td class="table-td text-right font-mono text-[11px] tabular-nums text-muted">
              {{
                point.disk_usage_percent === null || point.disk_usage_percent === undefined
                  ? '—'
                  : `${point.disk_usage_percent.toFixed(0)}%`
              }}
            </td>
            <td class="table-td font-mono text-[10.5px] text-faint">v{{ point.agent_version }}</td>
            <td class="table-td pr-5 text-right text-[10.5px] text-faint">
              {{ formatRelative(point.received_at) }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <EmptyState
      v-else
      title="没有历史快照"
      description="room-agent 成功上报后，这里会按采集时间累积历史。"
      icon="cpu"
    />
  </section>
</template>
