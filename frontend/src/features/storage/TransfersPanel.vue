<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api, isForbidden } from '@/api'
import type { Room, Transfer } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import ProgressBar from '@/components/ui/ProgressBar.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import {
  canRetryTransfer,
  transferDirectionLabel,
  transferStatusMeta,
} from '@/features/storage/labels'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatDateTime, shortId } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const props = defineProps<{
  rooms: Room[]
  roomsLoading: boolean
  roomsError: string
}>()

const auth = useAuthStore()
const toast = useToastStore()

const PAGE_SIZE = 20

const items = ref<Transfer[]>([])
const page = ref(1)
const total = ref(0)
const loading = ref(false)
const error = ref('')
const roomId = ref('')

const pendingRetry = ref<Transfer | null>(null)
const retrying = ref(false)
const retryError = ref('')

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const canManage = computed(() => auth.can(PERMISSIONS.storageManage))
const confirmOpen = computed(() => pendingRetry.value !== null)

function roomLabel(id: string | null | undefined): string {
  if (!id) return '—'
  const room = props.rooms.find((item) => item.id === id)
  return room ? `${room.name} · ${room.code}` : shortId(id)
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const data = await api.listTransfers({
      room_id: roomId.value || undefined,
      page: page.value,
      page_size: PAGE_SIZE,
    })
    items.value = data.items ?? []
    total.value = data.total ?? 0
    if (data.page) page.value = data.page
  } catch (caught) {
    items.value = []
    total.value = 0
    error.value = isForbidden(caught)
      ? '缺少 storage:read 权限，无法加载分发任务。'
      : errorMessage(caught, '加载分发任务失败')
  } finally {
    loading.value = false
  }
}

function onRoomChange() {
  page.value = 1
  void load()
}

function changePage(next: number) {
  page.value = next
  void load()
}

function requestRetry(transfer: Transfer) {
  retryError.value = ''
  pendingRetry.value = transfer
}

async function confirmRetry() {
  const target = pendingRetry.value
  if (!target?.id || retrying.value) return

  retrying.value = true
  retryError.value = ''
  try {
    await api.retryTransfer(target.id)
    pendingRetry.value = null
    toast.success('已重置为待分发', '分发任务进度已清零，等待节点重新拉取')
    await load()
  } catch (caught) {
    retryError.value = isForbidden(caught)
      ? '缺少 storage:manage 权限，无法重试分发任务。'
      : errorMessage(caught, '重试分发任务失败')
    toast.error('重试分发任务失败', retryError.value)
  } finally {
    retrying.value = false
  }
}

onMounted(() => void load())
</script>

<template>
  <PageSection>
    <template #header>
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 class="text-[13.5px] font-semibold text-ink">分发任务</h2>
          <p class="mt-0.5 text-[11px] text-muted">镜像下载与对象收集的分发记录，按创建时间倒序</p>
        </div>
        <div class="flex items-center gap-2">
          <select
            v-model="roomId"
            class="select w-[200px]"
            aria-label="按房间筛选分发任务"
            :disabled="roomsLoading"
            @change="onRoomChange"
          >
            <option value="">全部房间</option>
            <option v-for="room in rooms" :key="room.id" :value="room.id">
              {{ room.name }} · {{ room.code }}
            </option>
          </select>
          <IconButton
            icon="refresh"
            label="刷新分发任务"
            variant="outline"
            :loading="loading"
            @click="load"
          />
        </div>
      </div>
    </template>

    <p
      v-if="roomsError"
      class="border-b border-line bg-amber-50 px-5 py-2 text-[11px] text-amber-700"
    >
      {{ roomsError }}
    </p>

    <div v-if="error" class="flex items-center justify-between px-5 py-3">
      <p class="text-[11.5px] text-rose-700">{{ error }}</p>
      <AppButton size="sm" variant="ghost" @click="load">重试</AppButton>
    </div>

    <div
      v-if="retryError"
      class="mx-5 mt-3 flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] text-rose-700"
      role="alert"
    >
      <span>{{ retryError }}</span>
      <AppButton size="sm" variant="ghost" @click="retryError = ''">关闭</AppButton>
    </div>

    <div class="overflow-x-auto">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">任务</th>
            <th class="table-th">房间</th>
            <th class="table-th">方向</th>
            <th class="table-th">状态</th>
            <th class="table-th w-[150px]">进度</th>
            <th class="table-th">错误</th>
            <th class="table-th">更新时间</th>
            <th class="table-th pr-5">操作</th>
          </tr>
        </thead>

        <tbody v-if="loading">
          <tr v-for="row in 5" :key="row" class="border-b border-line/70 last:border-b-0">
            <td v-for="cell in 8" :key="cell" class="table-td" :class="cell === 1 ? 'pl-5' : ''">
              <div class="h-3 w-20 animate-pulse rounded bg-line" />
            </td>
          </tr>
        </tbody>

        <tbody v-else-if="items.length">
          <tr
            v-for="transfer in items"
            :key="transfer.id"
            class="border-b border-line/70 last:border-b-0 hover:bg-canvas/50"
          >
            <td class="table-td pl-5 font-mono text-[10.5px] text-faint">
              {{ shortId(transfer.id) }}
            </td>
            <td class="table-td text-[11px]">
              <p class="truncate text-ink">{{ roomLabel(transfer.room_id) }}</p>
              <p class="mt-0.5 font-mono text-[10px] text-faint">
                {{ transfer.image_id ? `镜像 ${shortId(transfer.image_id)}` : '对象任务' }}
              </p>
            </td>
            <td class="table-td text-[11px]">{{ transferDirectionLabel(transfer.direction) }}</td>
            <td class="table-td">
              <StateBadge
                :label="transferStatusMeta(transfer.status).label"
                :tone="transferStatusMeta(transfer.status).tone"
              />
            </td>
            <td class="table-td">
              <div class="flex items-center gap-2">
                <ProgressBar
                  class="w-24"
                  :value="transfer.progress ?? 0"
                  :tone="transferStatusMeta(transfer.status).progress"
                />
                <span class="font-mono text-[10px] text-faint">{{ transfer.progress ?? 0 }}%</span>
              </div>
            </td>
            <td class="table-td">
              <p
                v-if="transfer.error"
                class="max-w-[240px] truncate text-[10.5px] text-rose-600"
                :title="transfer.error"
              >
                {{ transfer.error }}
              </p>
              <span v-else class="text-[10.5px] text-faint">—</span>
            </td>
            <td class="table-td text-[11px]">
              {{ formatDateTime(transfer.updated_at ?? transfer.created_at) }}
            </td>
            <td class="table-td pr-5">
              <IconButton
                v-if="canManage && canRetryTransfer(transfer.status)"
                icon="rotate-ccw"
                label="重试分发任务"
                size="sm"
                variant="outline"
                :loading="retrying && pendingRetry?.id === transfer.id"
                @click="requestRetry(transfer)"
              />
              <span v-else class="text-[10.5px] text-faint">—</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <EmptyState
      v-if="!loading && !items.length && !error"
      title="暂无分发任务"
      description="把已发布镜像分配到房间，或触发对象收集后，分发任务会出现在这里。"
      icon="download"
    />

    <PaginationBar
      v-if="!loading && total > 0"
      :page="page"
      :total-pages="totalPages"
      :total="total"
      label="个任务"
      @change="changePage"
    />

    <template #footer>
      <span class="text-[10.5px] text-faint">
        failed / expired 的任务可以重试，重试后状态回到 pending 且进度清零。
      </span>
    </template>

    <ConfirmDialog
      :open="confirmOpen"
      title="重试分发任务"
      description="将把分发任务重置为 pending 并清零进度，等待节点重新拉取。"
      :detail="
        pendingRetry
          ? `任务 ${shortId(pendingRetry.id)} · 房间 ${roomLabel(pendingRetry.room_id)} · 当前进度 ${pendingRetry.progress ?? 0}%`
          : ''
      "
      confirm-label="确认重试"
      :loading="retrying"
      @confirm="confirmRetry"
      @close="pendingRetry = null"
    />
  </PageSection>
</template>
