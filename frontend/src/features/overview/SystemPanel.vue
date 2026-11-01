<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api } from '@/api'
import AppIcon from '@/components/ui/AppIcon.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useInterval } from '@/composables/useInterval'
import type { ReadyStatus } from '@/api/types'
import { formatRelative } from '@/utils/format'

const ready = ref<ReadyStatus | null>(null)
const reachable = ref(true)
const latency = ref<number | null>(null)
const checkedAt = ref<string | null>(null)

async function probe() {
  const startedAt = performance.now()
  try {
    ready.value = await api.ready()
    reachable.value = true
  } catch {
    reachable.value = false
    ready.value = null
  } finally {
    latency.value = Math.round(performance.now() - startedAt)
    checkedAt.value = new Date().toISOString()
  }
}

onMounted(() => void probe())
useInterval(() => void probe(), 30_000)

const overall = computed(() => {
  if (!reachable.value) return { tone: 'rose' as const, label: '后端不可达' }
  if (ready.value && (!ready.value.database || !ready.value.cache)) {
    return { tone: 'amber' as const, label: '依赖异常' }
  }
  return { tone: 'emerald' as const, label: '运行正常' }
})

const checks = computed(() => [
  { label: 'PostgreSQL', ok: ready.value?.database ?? false, target: '主数据库' },
  { label: 'Redis', ok: ready.value?.cache ?? false, target: '缓存 / 会话 / 限流' },
])
</script>

<template>
  <section class="rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">系统状态</h2>
        <p class="mt-0.5 text-[11px] text-muted">
          最近探测 {{ checkedAt ? formatRelative(checkedAt) : '—' }}
        </p>
      </div>
      <StateBadge :label="overall.label" :tone="overall.tone" :pulse="overall.tone === 'emerald'" />
    </header>

    <div class="divide-y divide-line/70">
      <div
        v-for="check in checks"
        :key="check.label"
        class="flex items-center justify-between px-5 py-3"
      >
        <div class="flex items-center gap-2.5">
          <span
            class="flex h-7 w-7 items-center justify-center rounded-md border border-line bg-canvas/60 text-faint"
          >
            <AppIcon :name="check.label === 'Redis' ? 'zap' : 'database'" :size="14" />
          </span>
          <div>
            <p class="text-[12px] font-medium text-ink">{{ check.label }}</p>
            <p class="text-[10.5px] text-faint">{{ check.target }}</p>
          </div>
        </div>
        <StateBadge
          :label="ready ? (check.ok ? '可用' : '不可用') : '未知'"
          :tone="ready ? (check.ok ? 'emerald' : 'rose') : 'slate'"
          :dot="false"
        />
      </div>

      <div class="px-5 py-3">
        <dl class="space-y-2 text-[11px]">
          <div class="flex items-center justify-between">
            <dt class="text-faint">健康检查延迟</dt>
            <dd class="font-mono text-[11px] text-ink-soft">
              {{ latency === null ? '—' : `${latency} ms` }}
            </dd>
          </div>
          <div class="flex items-center justify-between">
            <dt class="text-faint">实例 ID</dt>
            <dd class="font-mono text-[10.5px] text-ink-soft">
              {{ ready?.instance ? ready.instance.slice(0, 13) : '—' }}
            </dd>
          </div>
          <div class="flex items-center justify-between">
            <dt class="text-faint">就绪状态</dt>
            <dd class="font-mono text-[10.5px] text-ink-soft">{{ ready?.status ?? 'unknown' }}</dd>
          </div>
        </dl>
      </div>
    </div>
  </section>
</template>
