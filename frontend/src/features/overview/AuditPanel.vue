<script setup lang="ts">
import type { AuditLog } from '@/api/schema'
import EmptyState from '@/components/ui/EmptyState.vue'
import { formatRelative } from '@/utils/format'

defineProps<{
  logs: AuditLog[]
}>()

const actionTone: Record<string, string> = {
  create: 'text-emerald-600',
  update: 'text-sky-600',
  delete: 'text-rose-600',
  set: 'text-violet-600',
  assign: 'text-amber-600',
  publish: 'text-emerald-600',
  verify: 'text-sky-600',
}

function toneOf(action: string | undefined): string {
  if (!action) return 'text-muted'
  const suffix = action.split('.').pop() ?? ''
  return actionTone[suffix] ?? 'text-muted'
}
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">最近审计</h2>
        <p class="mt-0.5 text-[11px] text-muted">用户 / 角色 / 资源的关键变更</p>
      </div>
      <RouterLink :to="{ name: 'audit' }" class="text-[11px] text-accent hover:underline">
        全部日志
      </RouterLink>
    </header>

    <ul v-if="logs.length" class="divide-y divide-line/70">
      <li v-for="log in logs" :key="log.id" class="flex items-start gap-3 px-5 py-2.5">
        <span class="mt-1 h-1.5 w-1.5 shrink-0 rounded-full bg-accent/60" />
        <div class="min-w-0 flex-1">
          <p class="truncate text-[11.5px] text-ink">
            <span :class="toneOf(log.action)" class="font-mono text-[10.5px]">
              {{ log.action }}
            </span>
            <span class="text-muted"> · {{ log.resource_type }}</span>
          </p>
          <p class="mt-0.5 truncate text-[10.5px] text-faint">
            {{ log.actor_display_name || log.actor_username || 'system' }}
            <span v-if="log.resource_id" class="font-mono">
              · {{ log.resource_id.slice(0, 8) }}
            </span>
          </p>
        </div>
        <span class="shrink-0 text-[10px] text-faint">
          {{ formatRelative(log.created_at) }}
        </span>
      </li>
    </ul>

    <EmptyState
      v-else
      title="暂无审计记录"
      description="管理操作发生后会写入审计日志，需要 audit:read 权限查看。"
      icon="scroll"
    />
  </section>
</template>
