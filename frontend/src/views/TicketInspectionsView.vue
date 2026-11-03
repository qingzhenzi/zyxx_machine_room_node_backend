<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { api, isForbidden } from '@/api'
import type { InspectionPlanView } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
import InspectionPlanModal from '@/features/tickets/InspectionPlanModal.vue'
import InspectionRunsPanel from '@/features/tickets/InspectionRunsPanel.vue'
import {
  formatInspectionInterval,
  formatInspectionScope,
  formatNextRun,
  runStatusMeta,
  runTicketLink,
} from '@/features/tickets/inspectionMeta'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatRelative } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

interface ScopeOption {
  id: string
  name: string
}

const auth = useAuthStore()
const toast = useToastStore()

const plans = ref<InspectionPlanView[]>([])
const loading = ref(false)
const error = ref('')
const forbidden = ref(false)

const areas = ref<ScopeOption[]>([])
const rooms = ref<ScopeOption[]>([])

const modalOpen = ref(false)
const editing = ref<InspectionPlanView | null>(null)
const deleteTarget = ref<InspectionPlanView | null>(null)
const deleting = ref(false)
const runningId = ref<string | null>(null)
const togglingId = ref<string | null>(null)

const historyPlan = ref<InspectionPlanView | null>(null)
const globalRunsOpen = ref(false)

const canManage = computed(() =>
  auth.canAny(PERMISSIONS.ticketInspectionManage, PERMISSIONS.ticketManage),
)

const areaNames = computed(() => {
  const map: Record<string, string> = {}
  for (const area of areas.value) map[area.id] = area.name
  return map
})
const roomNames = computed(() => {
  const map: Record<string, string> = {}
  for (const room of rooms.value) map[room.id] = room.name
  return map
})

function scopeText(plan: InspectionPlanView): string {
  return formatInspectionScope(plan.scope_type, plan.scope_id, {
    areas: areaNames.value,
    rooms: roomNames.value,
  })
}

async function load() {
  loading.value = true
  error.value = ''
  forbidden.value = false
  try {
    const data = await api.listInspectionPlans()
    plans.value = data
  } catch (caught) {
    plans.value = []
    if (isForbidden(caught)) {
      forbidden.value = true
    } else {
      error.value = errorMessage(caught, '加载巡检计划失败')
    }
  } finally {
    loading.value = false
  }
}

/** 区域 / 房间仅用于范围展示与选择；无读权限时静默回退 UUID。 */
async function loadScopeOptions() {
  try {
    const data = await api.listAreas({ page: 1, page_size: 200, include_inactive: true })
    areas.value = (data.items ?? []).map((area) => ({ id: area.id, name: area.name }))
  } catch {
    areas.value = []
  }
  try {
    const data = await api.listRooms({ page: 1, page_size: 200 })
    rooms.value = (data.items ?? []).map((room) => ({ id: room.id, name: room.name }))
  } catch {
    rooms.value = []
  }
}

function openCreate() {
  editing.value = null
  modalOpen.value = true
}

function openEdit(plan: InspectionPlanView) {
  editing.value = plan
  modalOpen.value = true
}

function onSaved() {
  modalOpen.value = false
  void load()
}

function openHistory(plan: InspectionPlanView) {
  historyPlan.value = plan
}

async function toggleEnabled(plan: InspectionPlanView, enabled: boolean) {
  if (togglingId.value) return
  togglingId.value = plan.id
  try {
    await api.updateInspectionPlan(plan.id, { enabled })
    toast.success(enabled ? '巡检计划已启用' : '巡检计划已停用', plan.name)
    await load()
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '需要 ticket:inspection:manage 或 ticket:manage。')
    } else {
      toast.error('更新失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    togglingId.value = null
  }
}

async function runNow(plan: InspectionPlanView) {
  if (runningId.value) return
  runningId.value = plan.id
  try {
    const run = await api.runInspectionPlan(plan.id)
    const meta = runStatusMeta(run.status)
    toast.success(
      `巡检已执行：${meta.label}`,
      `检查 ${run.checked_machines} 台 · 异常 ${run.issue_machines} 台`,
    )
    historyPlan.value = plan
    await load()
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '需要 ticket:inspection:manage 或 ticket:manage。')
    } else {
      toast.error('巡检执行失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    runningId.value = null
  }
}

function askDelete(plan: InspectionPlanView) {
  deleteTarget.value = plan
}

async function confirmDelete() {
  const target = deleteTarget.value
  if (!target) return
  deleting.value = true
  try {
    await api.deleteInspectionPlan(target.id)
    toast.success('巡检计划已删除', target.name)
    deleteTarget.value = null
    await load()
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '需要 ticket:inspection:manage 或 ticket:manage。')
    } else {
      toast.error('删除失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    deleting.value = false
  }
}

function refresh() {
  void Promise.all([load(), loadScopeOptions()])
}

onMounted(() => {
  void load()
  void loadScopeOptions()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="巡检计划"
      description="按计划定期巡检机器健康快照：发现异常自动建工单，并在运行历史里保留检查机器数与关联工单。"
    >
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新巡检计划"
          variant="outline"
          :loading="loading"
          @click="refresh"
        />
        <AppButton v-if="canManage" icon="plus" variant="primary" @click="openCreate">
          新建计划
        </AppButton>
      </template>
    </PageHeader>

    <div class="mt-4 flex items-center justify-between gap-3">
      <p class="text-[11px] text-faint">
        共 {{ plans.length }} 个计划 · worker 按间隔执行 · 停用后仍可手动立即运行
      </p>
      <p class="text-[11px] text-faint">最近运行 / 下次运行按浏览器本地时间展示</p>
    </div>

    <div
      v-if="error && plans.length"
      role="alert"
      class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <section class="mt-3 overflow-hidden rounded-xl border border-line bg-surface">
      <table v-if="plans.length" class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th min-w-[180px] pl-5">名称</th>
            <th class="table-th w-[140px]">范围</th>
            <th class="table-th w-[110px]">间隔</th>
            <th class="table-th w-[76px]">启用</th>
            <th class="table-th w-[210px]">最近运行</th>
            <th class="table-th w-[110px]">下次运行</th>
            <th class="table-th w-[200px] pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="plan in plans"
            :key="plan.id"
            class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            :class="plan.enabled ? '' : 'opacity-70'"
          >
            <td class="table-td pl-5">
              <div class="text-[12px] font-medium text-ink">{{ plan.name }}</div>
              <div class="mt-0.5 text-[10px] text-faint">
                创建于 {{ formatRelative(plan.created_at) }}
              </div>
            </td>
            <td class="table-td text-[11.5px]">{{ scopeText(plan) }}</td>
            <td class="table-td text-[11.5px]">
              {{ formatInspectionInterval(plan.interval_hours) }}
            </td>
            <td class="table-td">
              <ToggleSwitch
                v-if="canManage"
                :model-value="plan.enabled"
                :disabled="togglingId !== null"
                @update:model-value="toggleEnabled(plan, $event)"
              />
              <StateBadge
                v-else
                :label="plan.enabled ? '启用' : '停用'"
                :tone="plan.enabled ? 'emerald' : 'slate'"
              />
            </td>
            <td class="table-td">
              <div v-if="plan.last_run" class="flex flex-wrap items-center gap-1.5">
                <StateBadge
                  :label="runStatusMeta(plan.last_run.status).label"
                  :tone="runStatusMeta(plan.last_run.status).tone"
                />
                <span
                  class="text-[10.5px]"
                  :class="plan.last_run.issue_machines > 0 ? 'text-rose-600' : 'text-faint'"
                >
                  异常 {{ plan.last_run.issue_machines }} 台
                </span>
                <span class="text-[10px] text-faint">
                  {{ formatRelative(plan.last_run.started_at) }}
                </span>
                <RouterLink
                  v-if="runTicketLink(plan.last_run.ticket_id)"
                  :to="{
                    name: 'ticket-detail',
                    params: { id: runTicketLink(plan.last_run.ticket_id) },
                  }"
                  class="link text-[10.5px]"
                >
                  工单
                </RouterLink>
              </div>
              <span v-else class="text-[11px] text-faint">尚未运行</span>
            </td>
            <td class="table-td text-[11px] text-faint">
              {{ formatNextRun(plan.next_run_at) }}
            </td>
            <td class="table-td pr-5" @click.stop>
              <div class="flex items-center justify-end gap-1">
                <AppButton
                  v-if="canManage"
                  size="sm"
                  icon="play-circle"
                  :loading="runningId === plan.id"
                  :disabled="runningId !== null && runningId !== plan.id"
                  @click="runNow(plan)"
                >
                  立即运行
                </AppButton>
                <IconButton
                  icon="clock"
                  label="查看运行历史"
                  size="sm"
                  @click="openHistory(plan)"
                />
                <IconButton
                  v-if="canManage"
                  icon="pencil"
                  label="编辑巡检计划"
                  size="sm"
                  @click="openEdit(plan)"
                />
                <IconButton
                  v-if="canManage"
                  icon="trash"
                  label="删除巡检计划"
                  size="sm"
                  @click="askDelete(plan)"
                />
              </div>
            </td>
          </tr>
        </tbody>
      </table>

      <div v-else-if="loading" class="space-y-3 px-5 py-6">
        <div v-for="row in 5" :key="row" class="h-9 animate-pulse rounded-md bg-canvas" />
      </div>

      <EmptyState
        v-else-if="forbidden"
        title="缺少巡检计划权限"
        description="当前账号既没有 ticket:inspection:manage 也没有 ticket:manage，无法查看巡检计划。"
        icon="lock"
      />

      <EmptyState v-else-if="error" title="加载巡检计划失败" :description="error" icon="alert">
        <template #action>
          <AppButton icon="refresh" @click="load()">重试</AppButton>
        </template>
      </EmptyState>

      <EmptyState
        v-else
        title="还没有巡检计划"
        description="巡检计划会按间隔检查范围内机器的健康快照，发现异常自动建工单；先从「新建计划」开始。"
        icon="rotate-cw"
      >
        <template v-if="canManage" #action>
          <AppButton icon="plus" variant="primary" @click="openCreate">新建计划</AppButton>
        </template>
      </EmptyState>
    </section>

    <PageSection class="mt-4" title="最近运行" description="所有巡检计划最近 20 条执行记录">
      <template #header>
        <div class="flex items-center justify-between gap-3">
          <div>
            <h2 class="text-[13.5px] font-semibold text-ink">最近运行</h2>
            <p class="mt-0.5 text-[11px] text-muted">所有巡检计划最近 20 条执行记录</p>
          </div>
          <AppButton size="sm" variant="ghost" @click="globalRunsOpen = !globalRunsOpen">
            {{ globalRunsOpen ? '收起' : '展开' }}
          </AppButton>
        </div>
      </template>
      <InspectionRunsPanel v-if="globalRunsOpen" :limit="20" />
      <p v-else class="px-5 py-4 text-[11px] text-faint">
        展开查看所有计划最近 20 条运行记录（含手动运行）。
      </p>
    </PageSection>

    <InspectionPlanModal
      :open="modalOpen"
      :plan="editing"
      :areas="areas"
      :rooms="rooms"
      @close="modalOpen = false"
      @saved="onSaved"
    />

    <AppModal
      :open="historyPlan !== null"
      :title="`运行历史 · ${historyPlan?.name ?? ''}`"
      description="该计划最近 20 次运行；异常机器数 > 0 时会自动关联一张机器健康工单。"
      size="xl"
      @close="historyPlan = null"
    >
      <InspectionRunsPanel v-if="historyPlan" :plan-id="historyPlan.id" :limit="20" />
    </AppModal>

    <ConfirmDialog
      :open="deleteTarget !== null"
      title="删除巡检计划"
      :description="`确认删除计划「${deleteTarget?.name ?? ''}」吗？`"
      detail="删除后 worker 不再自动执行该计划，历史运行记录会一起删除；已创建的工单不受影响。"
      confirm-label="删除计划"
      danger
      :loading="deleting"
      @close="deleteTarget = null"
      @confirm="confirmDelete"
    />
  </div>
</template>
