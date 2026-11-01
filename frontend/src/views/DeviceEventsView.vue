<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'

import { api, isForbidden, streamDeviceEvents } from '@/api'
import type { PresenceEvent } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import DeliveryPanel from '@/features/deviceEvents/DeliveryPanel.vue'
import LiveEventList from '@/features/deviceEvents/LiveEventList.vue'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, formatRelative } from '@/utils/format'

type StreamStatus = 'idle' | 'connecting' | 'live' | 'offline'

const HISTORY_PAGE = 50
const HISTORY_MAX = 200
const HISTORY_VISIBLE_MAX = 300

const events = ref<PresenceEvent[]>([])
const liveIds = ref<number[]>([])
const historyLoading = ref(false)
const historyError = ref('')
const limit = ref(HISTORY_PAGE)
const lastId = ref(0)
const deliveriesRefresh = ref(0)

const streamStatus = ref<StreamStatus>('idle')
const streamError = ref('')
const lastHeartbeatAt = ref<string | null>(null)

let controller: AbortController | null = null
let connectTimer: ReturnType<typeof setTimeout> | undefined
let disposed = false

const canLoadMore = computed(() => events.value.length >= limit.value && limit.value < HISTORY_MAX)

const streamMeta = computed<{ label: string; banner: string; dot: string }>(() => {
  switch (streamStatus.value) {
    case 'live':
      return {
        label: '实时流已连接',
        banner: 'border-emerald-200 bg-emerald-50 text-emerald-800',
        dot: 'bg-emerald-500',
      }
    case 'connecting':
      return {
        label: '正在连接实时流…',
        banner: 'border-sky-200 bg-sky-50 text-sky-800',
        dot: 'bg-sky-500 animate-pulse',
      }
    case 'offline':
      return {
        label: '实时流已断开',
        banner: 'border-rose-200 bg-rose-50 text-rose-700',
        dot: 'bg-rose-500',
      }
    default:
      return {
        label: '实时流未连接',
        banner: 'border-line bg-surface text-muted',
        dot: 'bg-slate-400',
      }
  }
})

async function loadHistory() {
  historyLoading.value = true
  historyError.value = ''
  try {
    const data = await api.listDeviceEvents({ limit: limit.value })
    const list = [...(data.items ?? [])].sort((left, right) => right.id - left.id)
    events.value = list
    lastId.value = list.reduce((max, event) => Math.max(max, event.id), lastId.value)
    const visibleIds = new Set(list.map((event) => event.id))
    liveIds.value = liveIds.value.filter((id) => visibleIds.has(id))
  } catch (caught) {
    historyError.value = isForbidden(caught)
      ? '缺少权限：当前账号没有设备读取权限（device:read）。'
      : errorMessage(caught, '加载在线事件失败')
  } finally {
    historyLoading.value = false
  }
}

async function loadMore() {
  if (limit.value >= HISTORY_MAX) return
  limit.value = Math.min(HISTORY_MAX, limit.value + HISTORY_PAGE)
  await loadHistory()
}

function refreshAll() {
  void loadHistory()
  deliveriesRefresh.value += 1
}

function clearConnectTimer() {
  if (connectTimer) {
    clearTimeout(connectTimer)
    connectTimer = undefined
  }
}

function stopStream() {
  clearConnectTimer()
  if (controller) {
    controller.abort()
    controller = null
  }
}

function onHeartbeat() {
  streamStatus.value = 'live'
  lastHeartbeatAt.value = new Date().toISOString()
}

function onStreamEvent(event: PresenceEvent) {
  streamStatus.value = 'live'
  lastId.value = Math.max(lastId.value, event.id)

  const nextLiveIds = new Set(liveIds.value)
  nextLiveIds.add(event.id)
  liveIds.value = [...nextLiveIds]

  const index = events.value.findIndex((item) => item.id === event.id)
  if (index >= 0) events.value.splice(index, 1)
  events.value.unshift(event)
  if (events.value.length > HISTORY_VISIBLE_MAX) {
    events.value.length = HISTORY_VISIBLE_MAX
  }
}

function connectStream() {
  stopStream()
  streamError.value = ''
  streamStatus.value = 'connecting'
  const ctrl = new AbortController()
  controller = ctrl
  connectTimer = setTimeout(() => {
    connectTimer = undefined
    if (!ctrl.signal.aborted && streamStatus.value === 'connecting') {
      streamStatus.value = 'live'
    }
  }, 1_200)

  void streamDeviceEvents({
    afterId: lastId.value > 0 ? lastId.value : null,
    signal: ctrl.signal,
    onEvent: onStreamEvent,
    onHeartbeat,
  })
    .then(() => {
      if (ctrl.signal.aborted) return
      streamStatus.value = 'offline'
      streamError.value = '实时流已关闭，可重新连接；重连会从最后事件之后补发漏掉的事件。'
    })
    .catch((caught: unknown) => {
      if (ctrl.signal.aborted) return
      streamStatus.value = 'offline'
      streamError.value = isForbidden(caught)
        ? '缺少权限：当前账号没有设备读取权限（device:read）。'
        : errorMessage(caught, '实时流连接失败')
    })
}

function disconnectManually() {
  stopStream()
  streamStatus.value = 'idle'
  streamError.value = ''
}

onMounted(async () => {
  await loadHistory()
  if (disposed) return
  connectStream()
})

onUnmounted(() => {
  disposed = true
  stopStream()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="设备在线事件"
      description="在线状态翻转历史、webhook 投递记录与 SSE 实时事件流。"
      :badge="streamMeta.label"
    >
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新历史事件"
          variant="outline"
          :loading="historyLoading"
          @click="refreshAll"
        />
        <AppButton
          v-if="streamStatus === 'offline'"
          icon="refresh"
          variant="primary"
          @click="connectStream"
        >
          重新连接
        </AppButton>
        <AppButton
          v-else-if="streamStatus === 'live' || streamStatus === 'connecting'"
          icon="x"
          @click="disconnectManually"
        >
          断开实时流
        </AppButton>
        <AppButton v-else icon="play" variant="primary" @click="connectStream">
          连接实时流
        </AppButton>
      </template>
    </PageHeader>

    <div
      class="mt-4 flex flex-wrap items-center justify-between gap-3 rounded-lg border px-4 py-2.5"
      :class="streamMeta.banner"
    >
      <div class="flex flex-wrap items-center gap-2 text-[11.5px]">
        <span class="h-2 w-2 shrink-0 rounded-full" :class="streamMeta.dot" />
        <span class="font-medium">{{ streamMeta.label }}</span>
        <span v-if="streamError">{{ streamError }}</span>
        <span v-else-if="lastHeartbeatAt" class="text-faint">
          最近心跳 {{ formatRelative(lastHeartbeatAt) }}
        </span>
        <span v-else class="text-faint">事件游标 #{{ lastId }}</span>
      </div>
      <div class="flex items-center gap-2 text-[10.5px] text-faint">
        <span v-if="streamStatus === 'live'">
          {{ lastHeartbeatAt ? formatFullDateTime(lastHeartbeatAt) : '已建立连接，等待事件或心跳' }}
        </span>
        <span>已加载 {{ events.length }} 条</span>
      </div>
    </div>

    <div class="mt-4 grid grid-cols-12 items-start gap-4">
      <div class="col-span-8">
        <PageSection>
          <template #header>
            <div class="flex items-start justify-between gap-3">
              <div>
                <h2 class="text-[13.5px] font-semibold text-ink">历史在线事件</h2>
                <p class="mt-0.5 text-[11px] text-muted">
                  按事件 id 倒序；实时流收到的新事件会插入顶部并标记「实时」。
                </p>
              </div>
            </div>
          </template>

          <LiveEventList
            :events="events"
            :live-ids="liveIds"
            :loading="historyLoading"
            :error="historyError"
            :can-load-more="canLoadMore"
            @load-more="loadMore"
            @retry="loadHistory"
          />
        </PageSection>
      </div>

      <div class="col-span-4">
        <DeliveryPanel :refresh-token="deliveriesRefresh" />
      </div>
    </div>
  </div>
</template>
