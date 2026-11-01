<script setup lang="ts">
import { computed, ref } from 'vue'

import type { ScheduleEventDelivery, ScheduleEventDetail } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import InfoList from '@/components/ui/InfoList.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'
import { triggerKindLabel, triggerKindTone } from '@/features/events/kinds'
import { formatDateTime } from '@/utils/format'

const props = withDefaults(
  defineProps<{
    detail?: ScheduleEventDetail | null
    loading?: boolean
    error?: string
    roomName?: string
    canManage?: boolean
    retryingDeliveryId?: number | null
    replaying?: boolean
  }>(),
  {
    detail: null,
    loading: false,
    error: '',
    roomName: '',
    canManage: false,
    retryingDeliveryId: null,
    replaying: false,
  },
)

const emit = defineEmits<{
  retry: [deliveryId: number]
  replay: []
  reload: []
}>()

const replayOpen = ref(false)

const event = computed(() => props.detail?.event ?? null)
const deliveries = computed(() => props.detail?.deliveries ?? [])

const infoItems = computed(() => {
  const value = event.value
  if (!value) return []
  return [
    { label: '事件类型', value: triggerKindLabel(value.event_kind) },
    { label: '房间', value: props.roomName || value.room_id || '—' },
    { label: '触发时间（房间本地）', value: formatDateTime(value.triggered_at_local) },
    { label: '时区', value: value.timezone ?? '—', mono: true },
    { label: '事件标题', value: value.title || '—' },
    { label: '触发时间（UTC）', value: formatDateTime(value.triggered_at) },
  ]
})

const deliveryStatusMeta: Record<string, { label: string; tone: BadgeTone }> = {
  pending: { label: '待投递', tone: 'amber' },
  delivered: { label: '已投递', tone: 'emerald' },
  failed: { label: '失败', tone: 'rose' },
  skipped: { label: '已跳过', tone: 'slate' },
}

const deliveryTargetMeta: Record<string, string> = {
  pubsub: '消息发布（pubsub）',
  downlink: '设备下行（downlink）',
  webhook: 'Webhook',
}

function statusMeta(status: string | undefined) {
  return deliveryStatusMeta[status ?? ''] ?? { label: status ?? '—', tone: 'slate' as const }
}

function targetLabel(target: ScheduleEventDelivery['target']) {
  return deliveryTargetMeta[target ?? ''] ?? target ?? '—'
}

function confirmReplay() {
  replayOpen.value = false
  emit('replay')
}
</script>

<template>
  <PageSection>
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div class="min-w-0">
          <h2 class="text-[13.5px] font-semibold text-ink">事件详情</h2>
          <p class="mt-0.5 truncate text-[11px] text-muted">
            {{ event?.title || '点击左侧事件查看 payload 与投递明细' }}
          </p>
        </div>
        <AppButton
          v-if="canManage && event"
          size="sm"
          icon="play-circle"
          :loading="replaying"
          :disabled="!deliveries.length"
          @click="replayOpen = true"
        >
          重放事件
        </AppButton>
      </div>
    </template>

    <div v-if="loading" class="space-y-3 px-5 py-4">
      <div v-for="index in 5" :key="index" class="h-8 animate-pulse rounded bg-line/60" />
    </div>

    <div
      v-else-if="error"
      class="m-5 flex items-center justify-between gap-3 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="emit('reload')">重试</AppButton>
    </div>

    <EmptyState
      v-else-if="!event"
      title="未选择事件"
      description="点击左侧事件列表中的任意一行，查看触发 payload 与投递明细。"
      icon="zap"
    />

    <div v-else class="space-y-4 px-5 py-4">
      <div class="flex flex-wrap items-center gap-2">
        <StateBadge
          :label="triggerKindLabel(event.event_kind)"
          :tone="triggerKindTone(event.event_kind)"
        />
        <StateBadge :label="`投递 ${deliveries.length} 条`" tone="slate" :dot="false" />
      </div>

      <InfoList :items="infoItems" :columns="2" />

      <dl class="grid grid-cols-2 gap-x-6 gap-y-3 border-t border-line/70 pt-3">
        <div class="min-w-0">
          <dt class="text-[10.5px] text-faint">事件键</dt>
          <dd class="mt-0.5">
            <CopyText :value="event.event_key" label="事件键" truncate />
          </dd>
        </div>
        <div class="min-w-0">
          <dt class="text-[10.5px] text-faint">事件 ID</dt>
          <dd class="mt-0.5">
            <CopyText :value="event.id" label="事件 ID" truncate />
          </dd>
        </div>
        <div class="min-w-0">
          <dt class="text-[10.5px] text-faint">课表 ID</dt>
          <dd class="mt-0.5">
            <CopyText :value="event.schedule_id" label="课表 ID" truncate />
          </dd>
        </div>
        <div class="min-w-0">
          <dt class="text-[10.5px] text-faint">规则 ID</dt>
          <dd class="mt-0.5">
            <CopyText :value="event.rule_id" label="规则 ID" truncate />
          </dd>
        </div>
      </dl>

      <div>
        <p class="mb-1.5 text-[11.5px] font-medium text-ink-soft">事件 payload</p>
        <JsonBlock :value="event.payload ?? {}" collapsible label="payload" />
      </div>

      <div class="border-t border-line/70 pt-3">
        <p class="mb-2 text-[11.5px] font-medium text-ink-soft">投递明细</p>

        <EmptyState
          v-if="!deliveries.length"
          title="没有投递记录"
          description="事件可能尚未生成投递目标，可稍后刷新重试。"
          icon="layers"
        />

        <div v-else class="overflow-x-auto rounded-md border border-line">
          <table class="w-full border-collapse text-left">
            <thead>
              <tr class="border-b border-line bg-canvas/50">
                <th class="table-th pl-3">目标</th>
                <th class="table-th">状态</th>
                <th class="table-th">尝试</th>
                <th class="table-th">最近错误</th>
                <th class="table-th">下次尝试</th>
                <th class="table-th pr-3 text-right">操作</th>
              </tr>
            </thead>
            <tbody>
              <tr
                v-for="delivery in deliveries"
                :key="delivery.id"
                class="border-b border-line/70 last:border-b-0"
              >
                <td class="table-td pl-3">
                  <span class="text-[11.5px] text-ink">{{ targetLabel(delivery.target) }}</span>
                </td>
                <td class="table-td">
                  <StateBadge
                    :label="statusMeta(delivery.status).label"
                    :tone="statusMeta(delivery.status).tone"
                  />
                </td>
                <td class="table-td font-mono text-[11px]">{{ delivery.attempts ?? 0 }}</td>
                <td class="table-td max-w-[200px]">
                  <span
                    class="line-clamp-2 text-[11px] text-rose-600"
                    :title="delivery.last_error ?? undefined"
                  >
                    {{ delivery.last_error || '—' }}
                  </span>
                </td>
                <td class="table-td text-[10.5px] text-faint">
                  {{ formatDateTime(delivery.next_attempt_at) }}
                </td>
                <td class="table-td pr-3 text-right">
                  <IconButton
                    v-if="canManage"
                    icon="rotate-ccw"
                    label="立即重试该投递"
                    size="sm"
                    :loading="retryingDeliveryId === delivery.id"
                    :disabled="
                      replaying ||
                      (retryingDeliveryId !== null && retryingDeliveryId !== delivery.id)
                    "
                    @click="emit('retry', delivery.id ?? 0)"
                  />
                  <span v-else class="text-[10.5px] text-faint">只读</span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </div>

    <ConfirmDialog
      :open="replayOpen"
      title="重放事件"
      :description="`将把事件「${event?.title ?? event?.id ?? ''}」的全部投递重置为待投递。`"
      detail="重放后事件会按投递退避策略尽快重新发送到 pubsub、设备下行与 webhook 目标。"
      confirm-label="重放事件"
      :loading="replaying"
      @confirm="confirmReplay"
      @close="replayOpen = false"
    />
  </PageSection>
</template>
