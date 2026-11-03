<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'

import { ApiError, api, isForbidden, isNotFound } from '@/api'
import type {
  TicketAssignee,
  TicketCategory,
  TicketDetail,
  TicketPriority,
  TransitionTicketRequest,
  UpdateTicketRequest,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import FormField from '@/components/ui/FormField.vue'
import InfoList from '@/components/ui/InfoList.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import TicketAttachments from '@/features/tickets/TicketAttachments.vue'
import TicketContextCard from '@/features/tickets/TicketContextCard.vue'
import TicketTimeline from '@/features/tickets/TicketTimeline.vue'
import {
  availableActions,
  isActiveStatus,
  isTerminalStatus,
  normalizeRootCause,
  rootCauseLabel,
  TICKET_CATEGORY_ORDER,
  TICKET_PRIORITY_ORDER,
  TICKET_ROOT_CAUSE_META,
  TICKET_ROOT_CAUSE_ORDER,
  ticketCategoryMeta,
  ticketPriorityMeta,
  ticketSourceMeta,
  ticketStatusMeta,
  type TicketAction,
} from '@/features/tickets/ticketMeta'
import { isOverdue } from '@/features/tickets/ticketStats'
import {
  eventMatchesTicket,
  ticketStreamStatusMeta,
  useTicketStream,
} from '@/features/tickets/useTicketStream'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatDateTime, formatRelative } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const auth = useAuthStore()
const toast = useToastStore()
const route = useRoute()
const router = useRouter()

const detail = ref<TicketDetail | null>(null)
const loading = ref(false)
const error = ref('')
const forbidden = ref(false)
const notFound = ref(false)

const context = computed(() => detail.value?.context ?? null)

const actionLoading = ref<string | null>(null)
const confirmAction = ref<TicketAction | null>(null)
const resolveOpen = ref(false)
const resolution = ref('')
const resolveError = ref('')
const resolveRootCause = ref('')

// 关闭工单弹窗（P1 Wave 2：关闭时可选记录根因分类）
const closeOpen = ref(false)
const closeRootCause = ref('')
const closeNote = ref('')

// 「工单设置」弹窗：转派处理人 / 改优先级 / 改分类（ticket:manage）。
const settingsOpen = ref(false)
const settingsSaving = ref(false)
const settingsLoading = ref(false)
const settingsError = ref('')
const settingsAssignees = ref<TicketAssignee[]>([])
const formAssignee = ref('')
const formPriority = ref<TicketPriority>('normal')
const formCategory = ref<TicketCategory>('other')

const commentBody = ref('')
const commenting = ref(false)

const ticketId = computed(() => String(route.params.id ?? ''))
const ticket = computed(() => detail.value?.ticket ?? null)

// 详情页实时刷新（可选）：只响应当前工单事件，400ms 防抖后静默重载。
const { status: streamStatus } = useTicketStream({
  matches: (event) => eventMatchesTicket(event, ticketId.value),
  onEvent: () => {
    void load(true)
  },
})
const streamMeta = computed(() => ticketStreamStatusMeta(streamStatus.value))
const events = computed(() => detail.value?.events ?? [])
const commentCount = computed(
  () => events.value.filter((event) => event.kind === 'commented').length,
)

// 细粒度权限（P1）：认领 / 评论 / 流转 / 转派 / 改单 可独立授权，
// 同时兼容旧的 ticket:handle / ticket:manage 粗粒度权限。
const canClaim = computed(() => auth.canAny(PERMISSIONS.ticketClaim, PERMISSIONS.ticketHandle))
const canTransition = computed(() =>
  auth.canAny(PERMISSIONS.ticketTransition, PERMISSIONS.ticketHandle),
)
const canHandle = computed(
  () => canClaim.value || canTransition.value || auth.can(PERMISSIONS.ticketHandle),
)
const actions = computed(() =>
  availableActions(ticket.value?.status, canHandle.value).filter((action) =>
    action.viaClaim || action.key === 'start' ? canClaim.value : canTransition.value,
  ),
)
const canAssign = computed(() => auth.canAny(PERMISSIONS.ticketAssign, PERMISSIONS.ticketManage))
const canUpdate = computed(() => auth.canAny(PERMISSIONS.ticketUpdate, PERMISSIONS.ticketManage))
const canEditSettings = computed(
  () =>
    ticket.value !== null &&
    !isTerminalStatus(ticket.value.status) &&
    (canAssign.value || canUpdate.value),
)
const settingsAssigneeOptions = computed(() => {
  const options: { id: string; label: string }[] = []
  const current = ticket.value
  if (
    current?.assignee_id &&
    !settingsAssignees.value.some((person) => person.id === current.assignee_id)
  ) {
    options.push({ id: current.assignee_id, label: current.assignee_name || current.assignee_id })
  }
  for (const person of settingsAssignees.value) {
    options.push({ id: person.id, label: person.display_name || person.id })
  }
  return options
})
const canComment = computed(
  () =>
    auth.canAny(PERMISSIONS.ticketComment, PERMISSIONS.ticketHandle) &&
    ticket.value !== null &&
    !isTerminalStatus(ticket.value.status),
)

const statusMeta = computed(() => ticketStatusMeta(ticket.value?.status))
const priorityMeta = computed(() => ticketPriorityMeta(ticket.value?.priority))
const categoryMeta = computed(() => ticketCategoryMeta(ticket.value?.category))
const sourceMeta = computed(() => ticketSourceMeta(ticket.value?.source))

const overdue = computed(() => (ticket.value ? isOverdue(ticket.value) : false))
const hasMetadata = computed(() => {
  const metadata = ticket.value?.metadata
  return (
    Boolean(metadata) && typeof metadata === 'object' && Object.keys(metadata as object).length > 0
  )
})
const occurrenceCount = computed(() => ticket.value?.occurrence_count ?? 0)

function ticketRootCause(): string | null {
  return ticket.value?.root_cause ?? null
}

const infoItems = computed(() => {
  const current = ticket.value
  if (!current) return []
  return [
    { label: '工单编号', value: current.code, mono: true },
    { label: '状态', value: statusMeta.value.label },
    {
      label: '优先级',
      value: `${priorityMeta.value.label} · SLA ${priorityMeta.value.slaHours} 小时`,
    },
    { label: '分类', value: categoryMeta.value.label },
    { label: '来源', value: sourceMeta.value.label },
    { label: '上报人', value: current.reporter_name ?? '—' },
    { label: '处理人', value: current.assignee_name ?? '未分配' },
    { label: '创建时间', value: formatDateTime(current.created_at) },
    { label: '更新时间', value: formatDateTime(current.updated_at) },
    { label: '解决时间', value: formatDateTime(current.resolved_at) },
    { label: 'SLA 时限', value: formatDateTime(current.due_at) },
    { label: '发生次数', value: `${occurrenceCount.value} 次` },
    ...(normalizeRootCause(ticketRootCause())
      ? [{ label: '根因分类', value: rootCauseLabel(ticketRootCause()) }]
      : []),
  ]
})

const confirmDescription = computed(() => {
  const current = ticket.value
  if (!current) return ''
  return `确认取消工单「${current.code}」吗？取消后不计入 SLA，但仍可在列表中重开。`
})

async function load(silent = false) {
  if (!ticketId.value) return
  if (!silent) loading.value = true
  error.value = ''
  forbidden.value = false
  notFound.value = false
  try {
    detail.value = await api.getTicket(ticketId.value)
  } catch (caught) {
    detail.value = null
    if (isNotFound(caught)) {
      notFound.value = true
    } else if (isForbidden(caught)) {
      forbidden.value = true
    } else {
      error.value = errorMessage(caught, '加载工单详情失败')
    }
  } finally {
    loading.value = false
  }
}

async function executeAction(
  action: TicketAction,
  extra?: { resolution?: string; rootCause?: string | null; note?: string },
) {
  const current = ticket.value
  if (!current) return
  actionLoading.value = action.key
  try {
    if (action.viaClaim) {
      await api.claimTicket(current.id)
    } else if (action.toStatus) {
      await api.transitionTicket(current.id, {
        to_status: action.toStatus,
        resolution: extra?.resolution,
        note: extra?.note,
        root_cause: (extra?.rootCause ?? null) as TransitionTicketRequest['root_cause'],
      })
    } else {
      return
    }
    toast.success(`${action.label}成功`, current.code)
    confirmAction.value = null
    resolveOpen.value = false
    closeOpen.value = false
    await load(true)
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '当前账号没有处理工单权限（ticket:handle）。')
    } else if (caught instanceof ApiError && caught.status === 409) {
      toast.error('工单状态已变化', '请查看最新状态后重试。')
      await load(true)
    } else {
      toast.error('操作失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    actionLoading.value = null
  }
}

function triggerAction(action: TicketAction) {
  if (action.key === 'close') {
    closeRootCause.value = ''
    closeNote.value = ''
    closeOpen.value = true
    return
  }
  if (action.needsConfirm) {
    confirmAction.value = action
    return
  }
  if (action.needsResolution) {
    resolution.value = ''
    resolveRootCause.value = ''
    resolveError.value = ''
    resolveOpen.value = true
    return
  }
  void executeAction(action)
}

function submitResolve() {
  if (!resolution.value.trim()) {
    resolveError.value = '请填写解决说明（后端要求 resolved 状态必须带说明）'
    return
  }
  const action = actions.value.find((item) => item.key === 'resolve')
  if (!action) return
  resolveError.value = ''
  void executeAction(action, {
    resolution: resolution.value.trim(),
    rootCause: resolveRootCause.value || null,
  })
}

function submitClose() {
  const action = actions.value.find((item) => item.key === 'close')
  if (!action) return
  void executeAction(action, {
    rootCause: closeRootCause.value || null,
    note: closeNote.value.trim() || undefined,
  })
}

function submitConfirm() {
  const action = confirmAction.value
  if (!action) return
  void executeAction(action)
}

async function loadSettingsAssignees() {
  settingsLoading.value = true
  try {
    settingsAssignees.value = await api.ticketAssignees()
  } catch {
    // 没有 ticket:handle 时拿不到可转派人员，保留「未分配 + 当前处理人」兜底选项。
    settingsAssignees.value = []
  } finally {
    settingsLoading.value = false
  }
}

function openSettings() {
  const current = ticket.value
  if (!current) return
  formAssignee.value = current.assignee_id ?? ''
  formPriority.value = current.priority
  formCategory.value = current.category
  settingsError.value = ''
  settingsOpen.value = true
  void loadSettingsAssignees()
}

async function saveSettings() {
  const current = ticket.value
  if (!current || settingsSaving.value) return
  settingsError.value = ''
  settingsSaving.value = true
  try {
    let changed = false
    if ((current.assignee_id ?? '') !== formAssignee.value) {
      await api.assignTicket(current.id, { assignee_id: formAssignee.value || null })
      changed = true
    }
    const patch: UpdateTicketRequest = {}
    if (current.priority !== formPriority.value) patch.priority = formPriority.value
    if (current.category !== formCategory.value) patch.category = formCategory.value
    if (Object.keys(patch).length > 0) {
      await api.updateTicket(current.id, patch)
      changed = true
    }
    if (!changed) {
      settingsOpen.value = false
      return
    }
    toast.success('工单设置已更新', current.code)
    settingsOpen.value = false
    await load(true)
  } catch (caught) {
    if (isForbidden(caught)) {
      settingsError.value =
        '缺少权限：转派需要 ticket:manage，调整优先级 / 分类同样需要 ticket:manage。'
    } else {
      settingsError.value = errorMessage(caught, '保存工单设置失败，请稍后重试')
    }
    // 部分成功时也重载详情，保证页面状态与后端一致。
    await load(true)
  } finally {
    settingsSaving.value = false
  }
}

async function submitComment() {
  const current = ticket.value
  const body = commentBody.value.trim()
  if (!current || !body) return
  commenting.value = true
  try {
    await api.commentTicket(current.id, { body })
    commentBody.value = ''
    toast.success('评论已提交')
    await load(true)
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '当前账号没有处理工单权限（ticket:handle）。')
    } else if (caught instanceof ApiError && caught.status === 409) {
      toast.error('工单状态已变化', '请查看最新状态后重试。')
      await load(true)
    } else {
      toast.error('评论失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    commenting.value = false
  }
}

function onCommentKeydown(event: KeyboardEvent) {
  if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
    event.preventDefault()
    void submitComment()
  }
}

watch(
  () => route.params.id,
  () => {
    detail.value = null
    void load()
  },
)

onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <div class="mb-3 flex items-center justify-between gap-3">
      <RouterLink
        :to="{ name: 'tickets' }"
        class="inline-flex items-center gap-1.5 text-[11.5px] text-muted transition-colors hover:text-ink"
      >
        <span aria-hidden="true">←</span>
        返回工单列表
      </RouterLink>
      <div v-if="ticket" class="flex items-center gap-2">
        <StateBadge
          :label="streamMeta.label"
          :tone="streamMeta.tone"
          :pulse="streamStatus === 'live'"
        />
        <span class="text-[10.5px] text-faint">
          最近更新 {{ formatRelative(ticket.updated_at) }}
          <template v-if="overdue"> · 已超 SLA</template>
        </span>
      </div>
    </div>

    <div v-if="notFound" class="rounded-xl border border-line bg-surface">
      <EmptyState title="工单不存在" description="链接可能已失效，或工单已被删除。" icon="ticket">
        <template #action>
          <AppButton size="sm" @click="router.push({ name: 'tickets' })">返回工单列表</AppButton>
        </template>
      </EmptyState>
    </div>

    <div v-else-if="forbidden" class="rounded-xl border border-line bg-surface">
      <EmptyState
        title="缺少工单权限"
        description="当前账号没有 ticket:read 权限，无法查看工单详情。请联系管理员授予工单读取权限。"
        icon="lock"
      >
        <template #action>
          <AppButton size="sm" @click="router.push({ name: 'tickets' })">返回工单列表</AppButton>
        </template>
      </EmptyState>
    </div>

    <div v-else-if="error && !detail" class="rounded-xl border border-line bg-surface">
      <EmptyState title="加载工单失败" :description="error" icon="alert">
        <template #action>
          <AppButton icon="refresh" @click="load()">重试</AppButton>
        </template>
      </EmptyState>
    </div>

    <template v-else-if="ticket">
      <PageHeader :title="ticket.code">
        <template #description>
          <span class="mr-2 text-[13px] font-medium text-ink">{{ ticket.title }}</span>
          <span>
            上报人 {{ ticket.reporter_name ?? '—' }} · 创建于
            {{ formatDateTime(ticket.created_at) }} · 处理人 {{ ticket.assignee_name ?? '未分配' }}
          </span>
        </template>
        <template #actions>
          <StateBadge
            :label="statusMeta.label"
            :tone="statusMeta.tone"
            :pulse="isActiveStatus(ticket.status)"
          />
          <StateBadge :label="priorityMeta.label" :tone="priorityMeta.tone" :dot="false" />
          <StateBadge :label="categoryMeta.label" :tone="categoryMeta.tone" :dot="false" />
          <StateBadge :label="`来源 · ${sourceMeta.label}`" :tone="sourceMeta.tone" :dot="false" />
          <span v-if="actions.length" class="mx-0.5 h-5 w-px bg-line" aria-hidden="true" />
          <AppButton
            v-for="action in actions"
            :key="action.key"
            :variant="action.variant"
            :icon="action.icon"
            :loading="actionLoading === action.key"
            :disabled="actionLoading !== null && actionLoading !== action.key"
            @click="triggerAction(action)"
          >
            {{ action.label }}
          </AppButton>
          <AppButton
            v-if="canEditSettings"
            variant="secondary"
            icon="sliders"
            @click="openSettings"
          >
            工单设置
          </AppButton>
        </template>
      </PageHeader>

      <div
        v-if="error"
        role="alert"
        class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
      >
        <span>{{ error }}</span>
        <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
      </div>

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <div class="col-span-8 flex min-w-0 flex-col gap-4">
          <PageSection
            title="问题描述"
            :description="ticket.source === 'auto' ? '来自自动告警信号' : '来自人工上报'"
          >
            <div class="px-5 py-4">
              <p class="whitespace-pre-wrap text-[12px] leading-6 text-ink-soft">
                {{ ticket.description || '未填写问题描述。' }}
              </p>

              <div
                class="mt-4 flex items-start gap-3 rounded-md border border-line bg-canvas/40 px-3 py-2.5"
              >
                <span
                  class="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-line bg-surface text-faint"
                >
                  <AppIcon :name="sourceMeta.icon" :size="13" />
                </span>
                <div class="min-w-0 flex-1">
                  <div class="flex flex-wrap items-center gap-2">
                    <span class="text-[11.5px] font-medium text-ink">
                      {{ ticket.source === 'auto' ? '自动触发信号' : '人工上报' }}
                    </span>
                    <span
                      v-if="occurrenceCount > 1"
                      class="rounded bg-amber-50 px-1.5 py-0.5 text-[9.5px] font-medium text-amber-700"
                    >
                      重复发生 {{ occurrenceCount }} 次
                    </span>
                  </div>
                  <p v-if="ticket.source === 'auto'" class="mt-1 text-[10.5px] text-muted">
                    信号来源
                    <span class="font-mono">{{ ticket.source_type || '—' }}</span>
                    <template v-if="ticket.source_id">
                      · 引用
                      <CopyText :value="ticket.source_id" display="source_id" />
                    </template>
                  </p>
                  <p v-if="ticket.last_seen_at" class="mt-0.5 text-[10.5px] text-faint">
                    最近触发 {{ formatDateTime(ticket.last_seen_at) }}
                  </p>
                </div>
              </div>

              <div v-if="hasMetadata" class="mt-3">
                <JsonBlock
                  :value="ticket.metadata"
                  label="触发信号 metadata"
                  collapsible
                  max-height="260px"
                />
              </div>
            </div>
          </PageSection>

          <PageSection title="处理时间线">
            <template #header>
              <h2 class="text-[13.5px] font-semibold text-ink">处理时间线</h2>
              <p class="mt-0.5 text-[11px] text-muted">
                共 {{ events.length }} 条记录 · {{ commentCount }} 条评论 · 按时间正序
              </p>
            </template>
            <TicketTimeline :events="events" />

            <div v-if="canComment" class="border-t border-line px-5 py-4">
              <textarea
                v-model="commentBody"
                class="textarea min-h-[92px]"
                placeholder="补充处理说明或评论；支持 ⌘ / Ctrl + Enter 提交"
                aria-label="工单评论"
                @keydown="onCommentKeydown"
              />
              <div class="mt-2 flex items-center justify-between">
                <span class="text-[10.5px] text-faint"> 评论会写入工单时间线并记录操作人 </span>
                <AppButton
                  size="sm"
                  variant="primary"
                  icon="scroll"
                  :loading="commenting"
                  :disabled="!commentBody.trim()"
                  @click="submitComment"
                >
                  提交评论
                </AppButton>
              </div>
            </div>
            <div v-else class="border-t border-line px-5 py-3 text-[11px] text-muted">
              {{
                canHandle
                  ? '工单已关闭 / 已取消，如需继续跟进请先重开工单。'
                  : '当前账号没有处理工单权限（ticket:handle），无法评论。'
              }}
            </div>
          </PageSection>

          <PageSection title="附件" description="截图、日志与复现材料；单个文件 ≤ 20 MiB">
            <TicketAttachments :ticket-id="ticket.id" />
          </PageSection>
        </div>

        <div class="col-span-4 flex min-w-0 flex-col gap-4">
          <PageSection title="工单信息" description="状态、处理人与关键时间点">
            <div class="px-5 py-4">
              <InfoList :items="infoItems" />
              <p
                v-if="ticket.resolution"
                class="mt-3 rounded-md border border-line bg-canvas/40 px-3 py-2 text-[11px] leading-5 text-ink-soft"
              >
                <span class="text-faint">解决说明：</span>{{ ticket.resolution }}
              </p>
            </div>
          </PageSection>

          <PageSection title="关联上下文" description="来自机器遥测与在线事件">
            <TicketContextCard :ticket="ticket" :context="context" />
          </PageSection>
        </div>
      </div>
    </template>

    <div v-else class="rounded-xl border border-line bg-surface">
      <div class="space-y-3 px-5 py-8">
        <div class="h-6 w-1/3 animate-pulse rounded bg-canvas" />
        <div class="h-24 animate-pulse rounded bg-canvas" />
        <div class="h-40 animate-pulse rounded bg-canvas" />
      </div>
    </div>

    <AppModal
      :open="resolveOpen"
      title="标记已解决"
      description="解决说明会写入处理时间线，建议写清处理动作与验证结果。"
      @close="resolveOpen = false"
    >
      <div class="flex flex-col gap-4">
        <FormField
          label="根因分类"
          input-id="ticket-resolve-root-cause"
          hint="可选；关闭前仍可调整"
        >
          <select id="ticket-resolve-root-cause" v-model="resolveRootCause" class="select">
            <option value="">暂不标记</option>
            <option v-for="key in TICKET_ROOT_CAUSE_ORDER" :key="key" :value="key">
              {{ TICKET_ROOT_CAUSE_META[key].label }}
            </option>
          </select>
        </FormField>
        <FormField label="解决说明" input-id="ticket-resolution" required :error="resolveError">
          <textarea
            id="ticket-resolution"
            v-model="resolution"
            class="textarea min-h-[110px]"
            placeholder="例如：已重启 room-agent，GPU 温度回落到 65°C，连续 10 分钟正常。"
            data-autofocus
          />
        </FormField>
      </div>
      <template #footer>
        <AppButton :disabled="actionLoading !== null" @click="resolveOpen = false">取消</AppButton>
        <AppButton variant="primary" :loading="actionLoading === 'resolve'" @click="submitResolve">
          确认解决
        </AppButton>
      </template>
    </AppModal>

    <AppModal
      :open="settingsOpen"
      title="工单设置"
      description="变更处理人、优先级与分类；后端会写入工单时间线与审计日志。"
      @close="settingsOpen = false"
    >
      <div class="flex flex-col gap-4">
        <FormField
          label="处理人"
          input-id="ticket-settings-assignee"
          :hint="
            canAssign
              ? '选择「未分配」可释放当前处理人；转派目标必须拥有工单处理权限。'
              : '当前账号没有转派权限（ticket:assign / ticket:manage）。'
          "
        >
          <select
            id="ticket-settings-assignee"
            v-model="formAssignee"
            class="select"
            :disabled="settingsLoading || settingsSaving || !canAssign"
            data-autofocus
          >
            <option value="">未分配</option>
            <option v-for="person in settingsAssigneeOptions" :key="person.id" :value="person.id">
              {{ person.label }}
            </option>
          </select>
        </FormField>

        <div class="grid grid-cols-2 gap-3">
          <FormField label="优先级" input-id="ticket-settings-priority">
            <select
              id="ticket-settings-priority"
              v-model="formPriority"
              class="select"
              :disabled="settingsSaving || !canUpdate"
            >
              <option v-for="key in TICKET_PRIORITY_ORDER" :key="key" :value="key">
                {{ ticketPriorityMeta(key).label }}（SLA
                {{ ticketPriorityMeta(key).slaHours }} 小时）
              </option>
            </select>
          </FormField>
          <FormField label="分类" input-id="ticket-settings-category">
            <select
              id="ticket-settings-category"
              v-model="formCategory"
              class="select"
              :disabled="settingsSaving || !canUpdate"
            >
              <option v-for="key in TICKET_CATEGORY_ORDER" :key="key" :value="key">
                {{ ticketCategoryMeta(key).label }}
              </option>
            </select>
          </FormField>
        </div>

        <p v-if="settingsLoading" class="text-[10.5px] text-faint">正在加载可转派人员…</p>
        <p
          v-if="settingsError"
          role="alert"
          class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
        >
          {{ settingsError }}
        </p>
      </div>

      <template #footer>
        <AppButton :disabled="settingsSaving" @click="settingsOpen = false">取消</AppButton>
        <AppButton variant="primary" :loading="settingsSaving" @click="saveSettings">
          保存设置
        </AppButton>
      </template>
    </AppModal>

    <AppModal
      :open="closeOpen"
      title="关闭工单"
      description="确认问题已解决；根因分类用于后续分析，可留空。关闭后同类问题再次发生会创建新工单。"
      @close="closeOpen = false"
    >
      <div class="flex flex-col gap-4">
        <FormField
          label="根因分类"
          input-id="ticket-close-root-cause"
          hint="可选，写入工单供后续统计"
        >
          <select id="ticket-close-root-cause" v-model="closeRootCause" class="select">
            <option value="">暂不标记</option>
            <option v-for="key in TICKET_ROOT_CAUSE_ORDER" :key="key" :value="key">
              {{ TICKET_ROOT_CAUSE_META[key].label }}
            </option>
          </select>
        </FormField>
        <FormField label="关闭备注" input-id="ticket-close-note" hint="可选，会写入处理时间线">
          <textarea
            id="ticket-close-note"
            v-model="closeNote"
            class="textarea min-h-[90px]"
            placeholder="例如：与用户确认问题已恢复，连续观察 24 小时正常。"
          />
        </FormField>
      </div>
      <template #footer>
        <AppButton :disabled="actionLoading !== null" @click="closeOpen = false">取消</AppButton>
        <AppButton variant="primary" :loading="actionLoading === 'close'" @click="submitClose">
          确认关闭
        </AppButton>
      </template>
    </AppModal>

    <ConfirmDialog
      :open="confirmAction !== null"
      title="取消工单"
      :description="confirmDescription"
      confirm-label="取消工单"
      danger
      :loading="actionLoading !== null"
      @close="confirmAction = null"
      @confirm="submitConfirm"
    />
  </div>
</template>
