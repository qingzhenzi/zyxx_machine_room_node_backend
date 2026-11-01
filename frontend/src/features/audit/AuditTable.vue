<script setup lang="ts">
import { ref } from 'vue'

import type { AuditLog } from '@/api/schema'
import AppIcon from '@/components/ui/AppIcon.vue'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { actionLabel, resourceLabel } from '@/features/audit/auditOptions'
import { formatFullDateTime, shortId } from '@/utils/format'

defineProps<{
  logs: AuditLog[]
  loading: boolean
}>()

const expanded = ref<string[]>([])

const ACTION_TONES: Record<string, string> = {
  create: 'text-emerald-600',
  update: 'text-sky-600',
  delete: 'text-rose-600',
  set: 'text-violet-600',
  assign: 'text-amber-600',
  publish: 'text-emerald-600',
  verify: 'text-sky-600',
  retry: 'text-amber-600',
  replay: 'text-violet-600',
  bind: 'text-sky-600',
  unbind: 'text-rose-600',
  issue: 'text-amber-600',
  register: 'text-sky-600',
  presence: 'text-slate-500',
}

function toneOf(action: string | undefined): string {
  if (!action) return 'text-muted'
  const parts = action.split('.')
  const suffix = parts[parts.length - 1] ?? ''
  return ACTION_TONES[suffix] ?? 'text-muted'
}

function toggle(id: string | undefined) {
  if (!id) return
  const index = expanded.value.indexOf(id)
  if (index >= 0) expanded.value.splice(index, 1)
  else expanded.value.push(id)
}

function isExpanded(id: string | undefined): boolean {
  return !!id && expanded.value.includes(id)
}
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <table class="w-full border-collapse text-left">
      <thead>
        <tr class="border-b border-line bg-canvas/50">
          <th class="table-th w-8 pl-5" />
          <th class="table-th">操作</th>
          <th class="table-th">操作者</th>
          <th class="table-th">资源</th>
          <th class="table-th">IP</th>
          <th class="table-th">时间</th>
          <th class="table-th pr-5 text-right">详情</th>
        </tr>
      </thead>
      <tbody>
        <template v-if="loading">
          <tr
            v-for="row in 6"
            :key="`skeleton-${row}`"
            class="border-b border-line/70 last:border-b-0"
          >
            <td v-for="col in 7" :key="col" class="table-td">
              <span class="block h-3 animate-pulse rounded bg-canvas" />
            </td>
          </tr>
        </template>

        <template v-else>
          <template v-for="log in logs" :key="log.id">
            <tr
              class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
              :class="isExpanded(log.id) ? 'bg-canvas/40' : ''"
              @click="toggle(log.id)"
            >
              <td class="table-td pl-5">
                <AppIcon
                  :name="isExpanded(log.id) ? 'chevron-down' : 'chevron-right'"
                  :size="13"
                  class="text-faint"
                />
              </td>
              <td class="table-td">
                <div class="flex items-center gap-2">
                  <span
                    class="h-1.5 w-1.5 shrink-0 rounded-full bg-current opacity-70"
                    :class="toneOf(log.action)"
                  />
                  <div class="min-w-0">
                    <p class="font-mono text-[11px]" :class="toneOf(log.action)">
                      {{ log.action }}
                    </p>
                    <p class="truncate text-[10px] text-faint">{{ actionLabel(log.action) }}</p>
                  </div>
                </div>
              </td>
              <td class="table-td">
                <p class="text-[11.5px] text-ink-soft">
                  {{ log.actor_display_name || log.actor_username || 'system' }}
                </p>
                <p v-if="log.actor_username" class="font-mono text-[10px] text-faint">
                  {{ log.actor_username }}
                </p>
              </td>
              <td class="table-td">
                <div class="flex items-center gap-2">
                  <StateBadge :label="resourceLabel(log.resource_type)" tone="sky" :dot="false" />
                  <span class="font-mono text-[10.5px] text-faint" :title="log.resource_id ?? ''">
                    {{ shortId(log.resource_id, 8) }}
                  </span>
                </div>
              </td>
              <td class="table-td">
                <span class="font-mono text-[10.5px] text-muted">{{ log.ip || '—' }}</span>
              </td>
              <td class="table-td whitespace-nowrap">
                <span class="text-[11px] text-muted" :title="log.created_at">
                  {{ formatFullDateTime(log.created_at) }}
                </span>
              </td>
              <td class="table-td pr-5 text-right">
                <span class="text-[10.5px] text-faint">
                  {{ isExpanded(log.id) ? '收起' : '展开' }}
                </span>
              </td>
            </tr>

            <tr v-if="isExpanded(log.id)" class="border-b border-line/70 bg-canvas/30">
              <td />
              <td colspan="6" class="px-3 pb-3 pt-1">
                <div class="grid grid-cols-[1fr_260px] gap-4">
                  <JsonBlock :value="log.detail ?? {}" label="detail" />
                  <div class="space-y-2 rounded-md border border-line bg-surface px-3 py-2.5">
                    <div>
                      <p class="text-[10px] text-faint">审计 ID</p>
                      <CopyText :value="log.id" :display="shortId(log.id, 16)" label="审计 ID" />
                    </div>
                    <div>
                      <p class="text-[10px] text-faint">资源 ID</p>
                      <CopyText
                        :value="log.resource_id"
                        :display="shortId(log.resource_id, 16)"
                        label="资源 ID"
                      />
                    </div>
                    <div>
                      <p class="text-[10px] text-faint">操作者 ID</p>
                      <CopyText
                        :value="log.actor_id"
                        :display="shortId(log.actor_id, 16)"
                        label="操作者 ID"
                      />
                    </div>
                    <div>
                      <p class="text-[10px] text-faint">完整时间</p>
                      <p class="font-mono text-[10.5px] text-ink-soft">
                        {{ formatFullDateTime(log.created_at) }}
                      </p>
                    </div>
                  </div>
                </div>
              </td>
            </tr>
          </template>
        </template>
      </tbody>
    </table>

    <EmptyState
      v-if="!loading && !logs.length"
      title="没有匹配的审计记录"
      description="调整操作或资源类型筛选；管理操作发生后会写入审计日志。"
      icon="scroll"
    />

    <slot name="footer" />
  </section>
</template>
