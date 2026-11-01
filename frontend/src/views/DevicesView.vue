<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import { api, isForbidden } from '@/api'
import type { Area, DeviceInfo, DeviceStatus, Room } from '@/api/schema'
import type { DeviceListParams } from '@/api/types'
import AppButton from '@/components/ui/AppButton.vue'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import {
  DEVICE_STATUS_FILTERS,
  deviceOnlineMeta,
  deviceStatusMeta,
} from '@/features/devices/deviceMeta'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, formatRelative, shortId } from '@/utils/format'

const PAGE_SIZE = 20

const router = useRouter()

const items = ref<DeviceInfo[]>([])
const areas = ref<Area[]>([])
const rooms = ref<Room[]>([])
const total = ref(0)
const page = ref(1)
const keyword = ref('')
const statusFilter = ref('')
const assignedFilter = ref('')
const areaFilter = ref('')
const roomFilter = ref('')
const loading = ref(false)
const error = ref('')

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const hasFilter = computed(() =>
  Boolean(
    keyword.value.trim() ||
    statusFilter.value ||
    assignedFilter.value ||
    areaFilter.value ||
    roomFilter.value,
  ),
)

async function load() {
  loading.value = true
  error.value = ''
  const params: DeviceListParams = { page: page.value, page_size: PAGE_SIZE }
  const trimmed = keyword.value.trim()
  if (trimmed) params.keyword = trimmed
  if (statusFilter.value) params.status = statusFilter.value as DeviceStatus
  if (assignedFilter.value === 'true') params.assigned = true
  if (assignedFilter.value === 'false') params.assigned = false
  if (areaFilter.value) params.area_id = areaFilter.value
  if (roomFilter.value) params.room_id = roomFilter.value

  try {
    const data = await api.listDevices(params)
    items.value = data.items ?? []
    total.value = data.total ?? 0
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少权限：当前账号没有设备读取权限（device:read）。'
      : errorMessage(caught, '加载设备列表失败')
  } finally {
    loading.value = false
  }
}

async function loadAreas() {
  try {
    const data = await api.listAreas({ page: 1, page_size: 100, include_inactive: true })
    areas.value = data.items ?? []
  } catch {
    areas.value = []
  }
}

async function loadRooms() {
  try {
    const data = await api.listRooms({
      area_id: areaFilter.value || undefined,
      page_size: 100,
      include_inactive: true,
    })
    rooms.value = data.items ?? []
  } catch {
    rooms.value = []
  }
}

function resetPageAndLoad() {
  if (page.value === 1) {
    void load()
  } else {
    page.value = 1
  }
}

const { run: debouncedSearch } = useDebounceFn(resetPageAndLoad, 320)

watch(keyword, () => debouncedSearch())
watch(page, () => void load())

function onAreaChange() {
  roomFilter.value = ''
  void loadRooms()
  resetPageAndLoad()
}

function resetFilters() {
  keyword.value = ''
  statusFilter.value = ''
  assignedFilter.value = ''
  areaFilter.value = ''
  roomFilter.value = ''
  page.value = 1
  void loadRooms()
  void load()
}

function openDevice(device: DeviceInfo) {
  void router.push({ name: 'device-detail', params: { serial: device.serial } })
}

onMounted(() => {
  void load()
  void loadAreas()
  void loadRooms()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="设备" description="设备注册状态、在线状态与房间绑定。">
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新设备列表"
          variant="outline"
          :loading="loading"
          @click="load"
        />
      </template>
    </PageHeader>

    <div class="mt-4 flex flex-wrap items-center gap-2">
      <SearchInput v-model="keyword" class="w-[240px]" placeholder="搜索序列号 / 槽位 / 房间" />
      <div class="w-[160px]">
        <select
          v-model="areaFilter"
          class="select"
          aria-label="按区域筛选设备"
          @change="onAreaChange"
        >
          <option value="">全部区域</option>
          <option v-for="area in areas" :key="area.id" :value="area.id">
            {{ area.name }}
          </option>
        </select>
      </div>
      <div class="w-[170px]">
        <select
          v-model="roomFilter"
          class="select"
          aria-label="按房间筛选设备"
          @change="resetPageAndLoad"
        >
          <option value="">全部房间</option>
          <option v-for="room in rooms" :key="room.id" :value="room.id">
            {{ room.name }}
          </option>
        </select>
      </div>
      <div class="w-[130px]">
        <select
          v-model="statusFilter"
          class="select"
          aria-label="按注册状态筛选设备"
          @change="resetPageAndLoad"
        >
          <option value="">全部状态</option>
          <option v-for="status in DEVICE_STATUS_FILTERS" :key="status.value" :value="status.value">
            {{ status.label }}
          </option>
        </select>
      </div>
      <div class="w-[130px]">
        <select
          v-model="assignedFilter"
          class="select"
          aria-label="按绑定情况筛选设备"
          @change="resetPageAndLoad"
        >
          <option value="">全部绑定</option>
          <option value="true">已绑定</option>
          <option value="false">未绑定</option>
        </select>
      </div>
      <AppButton v-if="hasFilter" size="sm" variant="ghost" @click="resetFilters">
        重置筛选
      </AppButton>
      <span class="ml-auto text-[11px] text-faint">共 {{ total }} 台设备</span>
    </div>

    <div
      v-if="error && items.length"
      role="alert"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <section class="mt-4 overflow-hidden rounded-xl border border-line bg-surface">
      <table v-if="items.length" class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">序列号</th>
            <th class="table-th">槽位</th>
            <th class="table-th">房间</th>
            <th class="table-th">区域</th>
            <th class="table-th">注册状态</th>
            <th class="table-th">在线状态</th>
            <th class="table-th">最后心跳</th>
            <th class="table-th pr-5 text-right">详情</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="device in items"
            :key="device.serial"
            class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            @click="openDevice(device)"
          >
            <td class="table-td pl-5" @click.stop>
              <span :title="device.serial">
                <CopyText
                  :value="device.serial"
                  :display="shortId(device.serial, 20)"
                  label="序列号"
                />
              </span>
            </td>
            <td class="table-td font-mono text-[11px]">{{ device.device_slot }}</td>
            <td class="table-td">
              <div v-if="device.room_name" class="text-[12px] text-ink">
                {{ device.room_name }}
              </div>
              <div v-if="device.room_code" class="font-mono text-[10px] text-faint">
                {{ device.room_code }}
              </div>
              <span v-if="!device.room_name" class="text-faint">未绑定</span>
            </td>
            <td class="table-td">{{ device.area_name ?? '—' }}</td>
            <td class="table-td">
              <StateBadge
                :label="deviceStatusMeta(device.status).label"
                :tone="deviceStatusMeta(device.status).tone"
              />
            </td>
            <td class="table-td">
              <StateBadge
                :label="deviceOnlineMeta(device.online).label"
                :tone="deviceOnlineMeta(device.online).tone"
                :pulse="device.online === 'online'"
              />
            </td>
            <td class="table-td text-[11px] text-faint">
              <span :title="formatFullDateTime(device.last_seen_at)">
                {{ formatRelative(device.last_seen_at) }}
              </span>
            </td>
            <td class="table-td pr-5" @click.stop>
              <div class="flex items-center justify-end">
                <IconButton
                  icon="chevron-right"
                  label="查看设备详情"
                  size="sm"
                  @click="openDevice(device)"
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
        v-else-if="!error"
        :title="hasFilter ? '没有匹配的设备' : '还没有注册设备'"
        :description="
          hasFilter
            ? '试试调整区域、房间、状态或绑定情况筛选。'
            : '在房间详情中签发注册 token 后，设备完成注册会出现在这里。'
        "
        icon="monitor"
      />

      <EmptyState v-else title="加载设备失败" :description="error" icon="alert">
        <template #action>
          <AppButton icon="refresh" @click="load()">重试</AppButton>
        </template>
      </EmptyState>

      <PaginationBar
        v-if="!loading && items.length"
        :page="page"
        :total-pages="totalPages"
        :total="total"
        label="台设备"
        @change="(next) => (page = next)"
      />
    </section>
  </div>
</template>
