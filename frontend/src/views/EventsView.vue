<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api, isForbidden } from '@/api'
import type {
  EventRule,
  NewEventRule,
  Room,
  ScheduleEvent,
  ScheduleEventDetail,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import RoomRuleEditor from '@/features/events/RoomRuleEditor.vue'
import ScheduleEventDetailPanel from '@/features/events/ScheduleEventDetailPanel.vue'
import { TRIGGER_KINDS, triggerKindLabel, triggerKindTone } from '@/features/events/kinds'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatDateTime } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const PAGE_SIZE = 20

const auth = useAuthStore()
const toast = useToastStore()

const rooms = ref<Room[]>([])
const events = ref<ScheduleEvent[]>([])
const total = ref(0)
const page = ref(1)
const loading = ref(false)
const error = ref('')

const roomFilter = ref('')
const kindFilter = ref('')
const fromFilter = ref('')
const toFilter = ref('')

const selectedId = ref('')
const detail = ref<ScheduleEventDetail | null>(null)
const detailLoading = ref(false)
const detailError = ref('')
const retryingDeliveryId = ref<number | null>(null)
const replaying = ref(false)

const rulesRoomId = ref('')
const rules = ref<EventRule[]>([])
const rulesLoading = ref(false)
const rulesError = ref('')
const rulesSaving = ref(false)

const canManageEvents = computed(() => auth.can(PERMISSIONS.scheduleEventManage))
const canManageRules = computed(() => auth.can(PERMISSIONS.scheduleManage))
const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const roomNames = computed(() => new Map(rooms.value.map((room) => [room.id, room.name])))
const sortedRooms = computed(() =>
  [...rooms.value].sort((left, right) => left.name.localeCompare(right.name, 'zh-CN')),
)

function actionError(caught: unknown, fallback: string): string {
  return isForbidden(caught) ? '缺少触发事件管理权限' : errorMessage(caught, fallback)
}

function roomName(roomId: string | undefined): string {
  if (!roomId) return '—'
  return roomNames.value.get(roomId) ?? roomId.slice(0, 8)
}

function startOfDayIso(value: string): string | undefined {
  if (!value) return undefined
  const date = new Date(`${value}T00:00:00`)
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString()
}

function endOfDayIso(value: string): string | undefined {
  if (!value) return undefined
  const date = new Date(`${value}T23:59:59.999`)
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString()
}

async function loadRooms() {
  try {
    const result = await api.listRooms({ page_size: 100 })
    rooms.value = result.items ?? []
    const preferred = rooms.value.find((room) => room.is_active) ?? rooms.value[0]
    if (preferred) {
      rulesRoomId.value = preferred.id
      await loadRules()
    }
  } catch {
    rooms.value = []
  }
}

async function loadEvents() {
  loading.value = true
  error.value = ''
  try {
    const result = await api.listScheduleEvents({
      room_id: roomFilter.value || undefined,
      kind: kindFilter.value || undefined,
      from: startOfDayIso(fromFilter.value),
      to: endOfDayIso(toFilter.value),
      page: page.value,
      page_size: PAGE_SIZE,
    })
    events.value = result.items ?? []
    total.value = result.total ?? 0

    const selectedVisible = events.value.some((event) => event.id === selectedId.value)
    if (selectedVisible) {
      await loadDetail(selectedId.value, { silent: true })
    } else {
      const first = events.value[0]
      if (first?.id) {
        selectedId.value = first.id
        await loadDetail(first.id)
      } else {
        selectedId.value = ''
        detail.value = null
        detailError.value = ''
      }
    }
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少触发事件查看权限'
      : errorMessage(caught, '加载触发事件失败')
    events.value = []
    total.value = 0
    selectedId.value = ''
    detail.value = null
  } finally {
    loading.value = false
  }
}

function applyFilters() {
  page.value = 1
  void loadEvents()
}

function resetFilters() {
  roomFilter.value = ''
  kindFilter.value = ''
  fromFilter.value = ''
  toFilter.value = ''
  applyFilters()
}

function loadPage(next: number) {
  page.value = next
  void loadEvents()
}

async function selectEvent(event: ScheduleEvent) {
  if (!event.id) return
  selectedId.value = event.id
  await loadDetail(event.id)
}

async function loadDetail(id: string, options: { silent?: boolean } = {}) {
  if (!id) return
  if (!options.silent) {
    detailLoading.value = true
    detail.value = null
  }
  detailError.value = ''
  try {
    detail.value = await api.getScheduleEvent(id)
  } catch (caught) {
    detailError.value = isForbidden(caught)
      ? '缺少触发事件查看权限'
      : errorMessage(caught, '加载事件详情失败')
    if (!options.silent) detail.value = null
  } finally {
    detailLoading.value = false
  }
}

async function retryDelivery(deliveryId: number) {
  const eventId = selectedId.value
  if (!eventId || !deliveryId) return
  retryingDeliveryId.value = deliveryId
  try {
    await api.retryScheduleDelivery(eventId, deliveryId)
    toast.success('投递已重置', '该投递已重新加入待投递队列')
    await loadDetail(eventId, { silent: true })
  } catch (caught) {
    toast.error('重试投递失败', actionError(caught, '请稍后重试'))
  } finally {
    retryingDeliveryId.value = null
  }
}

async function replayEvent() {
  const eventId = selectedId.value
  if (!eventId) return
  replaying.value = true
  try {
    const result = await api.replayScheduleEvent(eventId)
    toast.success('事件已重放', `重置 ${result.reset ?? 0} 条投递为待投递`)
    await loadDetail(eventId, { silent: true })
  } catch (caught) {
    toast.error('重放事件失败', actionError(caught, '请稍后重试'))
  } finally {
    replaying.value = false
  }
}

async function loadRules() {
  if (!rulesRoomId.value) {
    rules.value = []
    return
  }
  rulesLoading.value = true
  rulesError.value = ''
  try {
    rules.value = await api.roomEventRules(rulesRoomId.value)
  } catch (caught) {
    rulesError.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载房间触发规则失败')
  } finally {
    rulesLoading.value = false
  }
}

function changeRulesRoom(value: string) {
  rulesRoomId.value = value
  void loadRules()
}

async function saveRules(next: NewEventRule[]) {
  if (!rulesRoomId.value) return
  rulesSaving.value = true
  try {
    rules.value = await api.replaceRoomEventRules(rulesRoomId.value, next)
    toast.success('触发规则已保存', `共 ${rules.value.length} 条规则`)
  } catch (caught) {
    toast.error('保存触发规则失败', actionError(caught, '请检查规则后重试'))
  } finally {
    rulesSaving.value = false
  }
}

onMounted(() => {
  void loadRooms()
  void loadEvents()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="触发事件"
      description="课表事件、投递明细、重放与重试；事件由房间触发规则命中排课实例后生成。"
    >
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新事件列表"
          variant="outline"
          :loading="loading"
          :disabled="loading"
          @click="loadEvents()"
        />
        <AppButton
          icon="refresh"
          :loading="detailLoading"
          :disabled="!selectedId"
          @click="loadDetail(selectedId)"
        >
          刷新详情
        </AppButton>
      </template>
    </PageHeader>

    <div
      v-if="error"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="loadEvents()">重试</AppButton>
    </div>

    <div class="mt-4 grid grid-cols-12 items-start gap-4">
      <div class="col-span-7">
        <PageSection>
          <template #header>
            <div class="flex items-start justify-between gap-4">
              <div>
                <h2 class="text-[13.5px] font-semibold text-ink">事件列表</h2>
                <p class="mt-0.5 text-[11px] text-muted">
                  共 {{ total }} 条 · 按触发时间倒序 · 点击行查看投递明细
                </p>
              </div>
            </div>
          </template>

          <div class="flex flex-wrap items-center gap-1.5 border-b border-line/70 px-4 py-3">
            <select
              v-model="roomFilter"
              class="select w-[132px]"
              aria-label="按房间筛选事件"
              @change="applyFilters"
            >
              <option value="">全部房间</option>
              <option v-for="room in sortedRooms" :key="room.id" :value="room.id">
                {{ room.name }}
              </option>
            </select>
            <select
              v-model="kindFilter"
              class="select w-[120px]"
              aria-label="按事件类型筛选"
              @change="applyFilters"
            >
              <option value="">全部类型</option>
              <option v-for="kind in TRIGGER_KINDS" :key="kind.value" :value="kind.value">
                {{ kind.label }}
              </option>
            </select>
            <label class="flex items-center gap-1.5 text-[11px] text-muted">
              从
              <input
                v-model="fromFilter"
                type="date"
                class="input input-sm w-[136px]"
                aria-label="触发时间起始日期"
              />
            </label>
            <label class="flex items-center gap-1.5 text-[11px] text-muted">
              到
              <input
                v-model="toFilter"
                type="date"
                class="input input-sm w-[136px]"
                aria-label="触发时间结束日期"
              />
            </label>
            <AppButton size="sm" variant="primary" :loading="loading" @click="applyFilters">
              查询
            </AppButton>
            <AppButton size="sm" variant="ghost" :disabled="loading" @click="resetFilters">
              清除
            </AppButton>
          </div>

          <div v-if="loading" class="px-5 py-3">
            <div
              v-for="index in 5"
              :key="index"
              class="mb-2 h-9 animate-pulse rounded bg-line/60"
            />
          </div>

          <EmptyState
            v-else-if="!events.length"
            title="还没有触发事件"
            description="当房间触发规则命中排课实例后事件会出现在这里。可先在下方维护触发规则，或检查课表是否已生成实例。"
            icon="zap"
          />

          <div v-else class="overflow-x-auto">
            <table class="w-full border-collapse text-left">
              <thead>
                <tr class="border-b border-line bg-canvas/50">
                  <th class="table-th pl-5">事件</th>
                  <th class="table-th">房间</th>
                  <th class="table-th">类型</th>
                  <th class="table-th">触发时间</th>
                  <th class="table-th pr-5">时区</th>
                </tr>
              </thead>
              <tbody>
                <tr
                  v-for="event in events"
                  :key="event.id"
                  class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0"
                  :class="selectedId === event.id ? 'bg-accent-soft/70' : 'hover:bg-canvas/50'"
                  @click="selectEvent(event)"
                >
                  <td class="table-td pl-5">
                    <p class="max-w-[240px] truncate text-[12px] font-medium text-ink">
                      {{ event.title || triggerKindLabel(event.event_kind) }}
                    </p>
                    <p class="max-w-[240px] truncate font-mono text-[10px] text-faint">
                      {{ event.event_key || event.id }}
                    </p>
                  </td>
                  <td class="table-td text-[11.5px]">{{ roomName(event.room_id) }}</td>
                  <td class="table-td">
                    <StateBadge
                      :label="triggerKindLabel(event.event_kind)"
                      :tone="triggerKindTone(event.event_kind)"
                    />
                  </td>
                  <td class="table-td text-[11px]">
                    {{ formatDateTime(event.triggered_at_local) }}
                  </td>
                  <td class="table-td pr-5 font-mono text-[10.5px] text-faint">
                    {{ event.timezone ?? '—' }}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>

          <PaginationBar
            v-if="!loading && total > 0"
            :page="page"
            :total-pages="totalPages"
            :total="total"
            label="条事件"
            @change="loadPage"
          />
        </PageSection>
      </div>

      <div class="col-span-5">
        <ScheduleEventDetailPanel
          :detail="detail"
          :loading="detailLoading"
          :error="detailError"
          :room-name="detail?.event?.room_id ? roomName(detail.event.room_id) : ''"
          :can-manage="canManageEvents"
          :retrying-delivery-id="retryingDeliveryId"
          :replaying="replaying"
          @retry="retryDelivery"
          @replay="replayEvent"
          @reload="loadDetail(selectedId)"
        />
      </div>
    </div>

    <RoomRuleEditor
      class="mt-4"
      :rooms="rooms"
      :room-id="rulesRoomId"
      :rules="rules"
      :loading="rulesLoading"
      :error="rulesError"
      :saving="rulesSaving"
      :can-manage="canManageRules"
      @update:room-id="changeRulesRoom"
      @reload="loadRules()"
      @save="saveRules"
    />
  </div>
</template>
