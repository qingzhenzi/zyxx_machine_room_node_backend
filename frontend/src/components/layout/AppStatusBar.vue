<script setup lang="ts">
import { computed, ref } from 'vue'

import { api } from '@/api'
import { useInterval } from '@/composables/useInterval'
import type { ReadyStatus } from '@/api/types'

const now = ref(new Date())
const ready = ref<ReadyStatus | null>(null)
const reachable = ref(true)
const latency = ref<number | null>(null)

async function probe() {
  const startedAt = performance.now()
  try {
    const data = await api.ready()
    ready.value = data
    reachable.value = true
  } catch {
    reachable.value = false
    ready.value = null
  } finally {
    latency.value = Math.round(performance.now() - startedAt)
  }
}

probe()
useInterval(() => {
  now.value = new Date()
}, 30_000)
useInterval(() => {
  void probe()
}, 30_000)

const clock = computed(() =>
  now.value.toLocaleTimeString('zh-CN', { hour12: false, hour: '2-digit', minute: '2-digit' }),
)

const timezone = computed(() => {
  const offset = -now.value.getTimezoneOffset() / 60
  return `UTC${offset >= 0 ? '+' : ''}${offset}`
})

const overall = computed(() => {
  if (!reachable.value) {
    return { tone: 'bg-rose-500', label: '后端不可达' }
  }
  if (ready.value && (!ready.value.database || !ready.value.cache)) {
    return { tone: 'bg-amber-500', label: '依赖异常' }
  }
  return { tone: 'bg-emerald-500', label: '系统运行正常' }
})
</script>

<template>
  <footer
    class="flex h-7 shrink-0 items-center justify-between border-t border-line bg-surface px-4 text-[11px] text-muted"
  >
    <div class="flex items-center gap-3.5">
      <span class="flex items-center gap-1.5">
        <span class="h-1.5 w-1.5 rounded-full" :class="overall.tone" />
        {{ overall.label }}
      </span>
      <span class="h-3 w-px bg-line" />
      <span class="flex items-center gap-2.5 font-mono text-[10px]">
        <span :class="ready?.database ? 'text-muted' : 'text-rose-600'">
          PG {{ ready ? (ready.database ? 'OK' : 'DOWN') : '—' }}
        </span>
        <span :class="ready?.cache ? 'text-muted' : 'text-rose-600'">
          Redis {{ ready ? (ready.cache ? 'OK' : 'DOWN') : '—' }}
        </span>
      </span>
      <span class="h-3 w-px bg-line" />
      <span class="font-mono text-[10px] text-muted">
        API {{ latency === null ? '—' : `${latency}ms` }}
      </span>
      <span
        class="rounded border border-line bg-canvas px-1.5 py-px font-mono text-[9px] text-faint"
      >
        {{ ready?.instance ? ready.instance.slice(0, 8) : 'NO-INSTANCE' }}
      </span>
    </div>
    <div class="flex items-center gap-3.5">
      <span>管理端</span>
      <span class="h-3 w-px bg-line" />
      <span>Room Node Console v0.1.0</span>
      <span class="h-3 w-px bg-line" />
      <span class="font-mono text-[10px]">{{ timezone }} {{ clock }}</span>
    </div>
  </footer>
</template>
