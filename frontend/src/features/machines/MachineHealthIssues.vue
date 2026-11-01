<script setup lang="ts">
import type { MachineIssueView } from './payload'
import { severityMeta } from './machineMeta'
import StateBadge from '@/components/ui/StateBadge.vue'

defineProps<{
  issues: MachineIssueView[]
}>()
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">健康问题</h2>
        <p class="mt-0.5 text-[11px] text-muted">来自 room-agent / platform-health 的判定与证据</p>
      </div>
      <span class="text-[11px] text-faint">{{ issues.length }} 项</span>
    </header>

    <ul v-if="issues.length" class="divide-y divide-line/70">
      <li v-for="issue in issues" :key="`${issue.code}-${issue.message}`" class="px-5 py-3">
        <div class="flex items-start gap-2.5">
          <StateBadge
            :label="severityMeta(issue.severity).label"
            :tone="severityMeta(issue.severity).tone"
            class="mt-0.5"
          />
          <div class="min-w-0 flex-1">
            <p class="text-[12px] font-medium text-ink">{{ issue.message }}</p>
            <p class="mt-0.5 font-mono text-[10.5px] text-faint">{{ issue.code }}</p>
            <p v-if="issue.detail" class="mt-1 text-[10.5px] leading-4 text-muted">
              {{ issue.detail }}
            </p>
          </div>
        </div>
      </li>
    </ul>

    <p v-else class="flex items-center gap-2 px-5 py-4 text-[12px] text-emerald-700">
      <span class="h-1.5 w-1.5 rounded-full bg-emerald-500" />
      未发现健康问题
    </p>
  </section>
</template>
