<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api, isForbidden } from '@/api'
import type { TicketRule } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import TicketRuleModal from '@/features/tickets/TicketRuleModal.vue'
import {
  formatRuleScope,
  formatRuleThresholds,
  signalMeta,
} from '@/features/tickets/ticketRuleMeta'
import { ticketPriorityMeta } from '@/features/tickets/ticketMeta'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatRelative, shortId } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

interface ScopeOption {
  id: string
  name: string
}

const auth = useAuthStore()
const toast = useToastStore()

const rules = ref<TicketRule[]>([])
const loading = ref(false)
const error = ref('')
const forbidden = ref(false)

const areas = ref<ScopeOption[]>([])
const rooms = ref<ScopeOption[]>([])

const modalOpen = ref(false)
const editing = ref<TicketRule | null>(null)
const deleteTarget = ref<TicketRule | null>(null)
const deleting = ref(false)

const canManage = computed(() =>
  auth.canAny(PERMISSIONS.ticketRuleManage, PERMISSIONS.ticketManage),
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

function scopeText(rule: TicketRule): string {
  const raw = formatRuleScope(rule.scope_type, rule.scope_id, {
    areas: areaNames.value,
    rooms: roomNames.value,
  })
  if (raw.includes(' · ') || raw === '全局') return raw
  return rule.scope_id ? `${raw} · ${shortId(rule.scope_id)}` : raw
}

async function load() {
  loading.value = true
  error.value = ''
  forbidden.value = false
  try {
    const data = await api.listTicketRules()
    rules.value = data.items ?? []
  } catch (caught) {
    rules.value = []
    if (isForbidden(caught)) {
      forbidden.value = true
    } else {
      error.value = errorMessage(caught, '加载自动规则失败')
    }
  } finally {
    loading.value = false
  }
}

/** 区域/房间名称仅用于「范围」列展示；ticket-only 角色无读权限时静默回退 UUID。 */
async function loadScopeOptions() {
  try {
    const data = await api.listAreas({ page: 1, page_size: 100, include_inactive: true })
    areas.value = (data.items ?? []).map((area) => ({ id: area.id, name: area.name }))
  } catch {
    areas.value = []
  }
  try {
    const data = await api.listRooms({ page: 1, page_size: 100 })
    rooms.value = (data.items ?? []).map((room) => ({ id: room.id, name: room.name }))
  } catch {
    rooms.value = []
  }
}

function openCreate() {
  editing.value = null
  modalOpen.value = true
}

function openEdit(rule: TicketRule) {
  editing.value = rule
  modalOpen.value = true
}

function onSaved() {
  modalOpen.value = false
  void load()
}

function askDelete(rule: TicketRule) {
  deleteTarget.value = rule
}

async function confirmDelete() {
  const target = deleteTarget.value
  if (!target) return
  deleting.value = true
  try {
    await api.deleteTicketRule(target.id)
    toast.success('自动规则已删除', target.name)
    deleteTarget.value = null
    void load()
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '需要 ticket:rule:manage 或 ticket:manage。')
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
      title="自动规则"
      description="自动建单规则：worker 每个 tick 读取启用的规则，按信号阈值决定是否建单；停用后该信号不再自动建单。"
    >
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新自动规则"
          variant="outline"
          :loading="loading"
          @click="refresh"
        />
        <AppButton v-if="canManage" icon="plus" variant="primary" @click="openCreate">
          新建规则
        </AppButton>
      </template>
    </PageHeader>

    <div class="mt-4 flex items-center justify-between gap-3">
      <p class="text-[11px] text-faint">
        共 {{ rules.length }} 条规则 · 阈值按信号类型生效 · 规则变更多副本下最多一个 tick 生效
      </p>
      <p class="text-[11px] text-faint">展示顺序：信号类型 → 名称</p>
    </div>

    <div
      v-if="error && rules.length"
      role="alert"
      class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <section class="mt-3 overflow-hidden rounded-xl border border-line bg-surface">
      <table v-if="rules.length" class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th min-w-[220px] pl-5">名称</th>
            <th class="table-th w-[150px]">信号</th>
            <th class="table-th w-[150px]">范围</th>
            <th class="table-th min-w-[160px]">阈值</th>
            <th class="table-th w-[88px]">优先级</th>
            <th class="table-th w-[80px]">启用</th>
            <th class="table-th w-[110px]">更新时间</th>
            <th class="table-th w-[96px] pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="rule in rules"
            :key="rule.id"
            class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            :class="rule.enabled ? '' : 'opacity-70'"
          >
            <td class="table-td pl-5">
              <div class="text-[12px] font-medium text-ink">{{ rule.name }}</div>
              <div class="mt-0.5 line-clamp-1 text-[10.5px] text-faint">
                {{ signalMeta(rule.signal_type).description }}
              </div>
            </td>
            <td class="table-td">
              <StateBadge
                :label="signalMeta(rule.signal_type).label"
                :tone="signalMeta(rule.signal_type).tone"
              />
            </td>
            <td class="table-td text-[11.5px]">{{ scopeText(rule) }}</td>
            <td class="table-td font-mono text-[11px] text-ink-soft">
              {{ formatRuleThresholds(rule.signal_type, rule.thresholds) }}
            </td>
            <td class="table-td">
              <StateBadge
                :label="ticketPriorityMeta(rule.priority).label"
                :tone="ticketPriorityMeta(rule.priority).tone"
                :dot="false"
              />
            </td>
            <td class="table-td">
              <StateBadge
                :label="rule.enabled ? '启用' : '停用'"
                :tone="rule.enabled ? 'emerald' : 'slate'"
              />
            </td>
            <td class="table-td text-[11px] text-faint">
              {{ formatRelative(rule.updated_at) }}
            </td>
            <td class="table-td pr-5" @click.stop>
              <div class="flex items-center justify-end gap-1">
                <IconButton
                  v-if="canManage"
                  icon="pencil"
                  label="编辑自动规则"
                  size="sm"
                  @click="openEdit(rule)"
                />
                <IconButton
                  v-if="canManage"
                  icon="trash"
                  label="删除自动规则"
                  size="sm"
                  @click="askDelete(rule)"
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
        title="缺少自动规则权限"
        description="当前账号既没有 ticket:rule:manage 也没有 ticket:manage，无法查看自动规则。"
        icon="lock"
      />

      <EmptyState v-else-if="error" title="加载自动规则失败" :description="error" icon="alert">
        <template #action>
          <AppButton icon="refresh" @click="load()">重试</AppButton>
        </template>
      </EmptyState>

      <EmptyState
        v-else
        title="还没有自动规则"
        description="worker 需要至少一条启用规则才会自动建单；可以按机器健康、设备离线、各类投递失败分别配置阈值。"
        icon="sliders"
      >
        <template v-if="canManage" #action>
          <AppButton icon="plus" variant="primary" @click="openCreate">新建规则</AppButton>
        </template>
      </EmptyState>
    </section>

    <TicketRuleModal
      :open="modalOpen"
      :rule="editing"
      :areas="areas"
      :rooms="rooms"
      @close="modalOpen = false"
      @saved="onSaved"
    />

    <ConfirmDialog
      :open="deleteTarget !== null"
      title="删除自动规则"
      :description="`确认删除规则「${deleteTarget?.name ?? ''}」吗？`"
      detail="删除后 worker 不再按该信号自动建单；已有工单不会被删除或关闭。"
      confirm-label="删除规则"
      danger
      :loading="deleting"
      @close="deleteTarget = null"
      @confirm="confirmDelete"
    />
  </div>
</template>
