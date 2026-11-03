<script setup lang="ts">
/**
 * 巡检运行历史（P1 Wave 3）：某计划的运行记录，或全局最近运行。
 * 列：开始 / 状态 / 检查机器 / 异常机器 / 关联工单 / 耗时。
 */
import { ref, watch } from 'vue'
import { RouterLink } from 'vue-router'

import { api, isForbidden } from '@/api'
import type { InspectionRun } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { errorMessage } from '@/utils/errors'
import { formatDateTime, shortId } from '@/utils/format'

import { formatRunDuration, runStatusMeta, runTicketLink } from './inspectionMeta'

const props = withDefaults(
  defineProps<{
    /** 传入则只看该计划；缺省 = 全局最近运行。 */
    planId?: string
    limit?: number
  }>(),
  { planId: '', limit: 20 },
)

const runs = ref<InspectionRun[]>([])
const loading = ref(false)
const error = ref('')

async function load() {
  loading.value = true
  error.value = ''
  try {
    const data = await api.listInspectionRuns({
      ...(props.planId ? { plan_id: props.planId } : {}),
      limit: props.limit,
    })
    runs.value = data
  } catch (caught) {
    runs.value = []
    if (isForbidden(caught)) {
      error.value = '缺少权限：需要 ticket:inspection:manage 或 ticket:manage。'
    } else {
      error.value = errorMessage(caught, '加载巡检运行记录失败')
    }
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.planId, props.limit],
  () => void load(),
  { immediate: true },
)
</script>

<template>
  <div class="px-5 py-4">
    <div v-if="loading" class="space-y-2">
      <div v-for="row in 3" :key="row" class="h-8 animate-pulse rounded-md bg-canvas" />
    </div>

    <div
      v-else-if="error"
      class="flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <p v-else-if="!runs.length" class="py-6 text-center text-[11px] text-faint">
      暂无运行记录{{ props.planId ? '' : '，计划到点或被手动执行后会出现在这里' }}。
    </p>

    <div v-else class="overflow-hidden rounded-md border border-line">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th v-if="!props.planId" class="table-th pl-3">计划</th>
            <th class="table-th" :class="props.planId ? 'pl-3' : ''">开始</th>
            <th class="table-th w-[88px]">状态</th>
            <th class="table-th w-[80px]">检查机器</th>
            <th class="table-th w-[80px]">异常机器</th>
            <th class="table-th w-[100px]">关联工单</th>
            <th class="table-th w-[90px] pr-3">耗时</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="run in runs"
            :key="run.id"
            class="border-b border-line/70 last:border-b-0"
            :title="run.error || undefined"
          >
            <td v-if="!props.planId" class="table-td pl-3 font-mono text-[10.5px] text-faint">
              {{ shortId(run.plan_id) }}
            </td>
            <td class="table-td text-[11px]" :class="props.planId ? 'pl-3' : ''">
              {{ formatDateTime(run.started_at) }}
            </td>
            <td class="table-td">
              <StateBadge
                :label="runStatusMeta(run.status).label"
                :tone="runStatusMeta(run.status).tone"
                :pulse="run.status === 'running'"
              />
            </td>
            <td class="table-td font-mono text-[11px]">{{ run.checked_machines }}</td>
            <td class="table-td font-mono text-[11px]">
              <span :class="run.issue_machines > 0 ? 'text-rose-600' : 'text-ink-soft'">
                {{ run.issue_machines }}
              </span>
            </td>
            <td class="table-td">
              <RouterLink
                v-if="runTicketLink(run.ticket_id)"
                :to="{ name: 'ticket-detail', params: { id: runTicketLink(run.ticket_id) } }"
                class="link text-[11px]"
              >
                查看工单
              </RouterLink>
              <span v-else class="text-[11px] text-faint">—</span>
            </td>
            <td class="table-td pr-3 text-[11px] text-faint">
              {{ formatRunDuration(run.started_at, run.finished_at) }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <p v-if="runs.length" class="mt-2 text-[10.5px] text-faint">
      最近 {{ runs.length }} 条运行记录{{ props.planId ? '' : '（全部计划）' }}。
    </p>
  </div>
</template>
