<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api, isForbidden } from '@/api'
import type { Area, Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import { deviceOnlineMeta, type StatusTone } from '@/features/devices/deviceMeta'
import RoomFormModal from '@/features/rooms/RoomFormModal.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatRelative } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const PAGE_SIZE = 20

const auth = useAuthStore()
const toast = useToastStore()
const route = useRoute()
const router = useRouter()

const items = ref<Room[]>([])
const areas = ref<Area[]>([])
const total = ref(0)
const page = ref(1)
const keyword = ref('')
const areaFilter = ref(typeof route.query.area_id === 'string' ? route.query.area_id : '')
const includeInactive = ref(false)
const loading = ref(false)
const error = ref('')

const formOpen = ref(false)
const editing = ref<Room | null>(null)
const deleteTarget = ref<Room | null>(null)
const deleting = ref(false)

const canManage = computed(() => auth.can(PERMISSIONS.roomManage))
const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const hasFilter = computed(() =>
  Boolean(keyword.value.trim() || areaFilter.value || includeInactive.value),
)

function slotLabel(room: Room): string {
  return room.device_slot ?? room.device?.device_slot ?? '—'
}

function onlineBadge(room: Room): { label: string; tone: StatusTone } {
  if (!room.device) return { label: '未绑定', tone: 'slate' }
  if (room.device.pending_token && !room.device.serial) return { label: '等待注册', tone: 'amber' }
  return deviceOnlineMeta(room.device.online)
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const data = await api.listRooms({
      page: page.value,
      page_size: PAGE_SIZE,
      area_id: areaFilter.value || undefined,
      keyword: keyword.value.trim() || undefined,
      include_inactive: includeInactive.value || undefined,
    })
    items.value = data.items ?? []
    total.value = data.total ?? 0
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少权限：当前账号没有房间读取权限（room:read）。'
      : errorMessage(caught, '加载房间列表失败')
  } finally {
    loading.value = false
  }
}

/** 区域下拉仅用于筛选/表单，失败时保留空列表不阻塞主列表。 */
async function loadAreas() {
  try {
    const data = await api.listAreas({ page: 1, page_size: 100, include_inactive: true })
    areas.value = data.items ?? []
  } catch {
    areas.value = []
  }
}

const { run: debouncedSearch } = useDebounceFn(() => {
  page.value = 1
  void load()
}, 320)

watch(keyword, () => debouncedSearch())
watch(areaFilter, () => {
  page.value = 1
  void load()
})
watch(includeInactive, () => {
  page.value = 1
  void load()
})
watch(page, () => void load())

function openCreate() {
  if (!areas.value.length) {
    toast.info('请先创建区域', '房间必须归属一个区域')
    return
  }
  editing.value = null
  formOpen.value = true
}

function openEdit(room: Room) {
  editing.value = room
  formOpen.value = true
}

function onSaved() {
  formOpen.value = false
  void load()
}

function openDetail(room: Room) {
  void router.push({ name: 'room-detail', params: { id: room.id } })
}

function askDelete(room: Room) {
  deleteTarget.value = room
}

function resetFilters() {
  keyword.value = ''
  areaFilter.value = ''
  includeInactive.value = false
  page.value = 1
  void load()
}

async function confirmDelete() {
  const target = deleteTarget.value
  if (!target) return
  deleting.value = true
  try {
    await api.deleteRoom(target.id)
    toast.success('房间已删除', target.name)
    deleteTarget.value = null
    if (items.value.length === 1 && page.value > 1) {
      page.value -= 1
    } else {
      void load()
    }
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '当前账号没有房间管理权限（room:manage）。')
    } else {
      toast.error('删除失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    deleting.value = false
  }
}

onMounted(() => {
  void load()
  void loadAreas()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="房间" description="房间、设备槽位与房间级 webhook。">
      <template #actions>
        <SearchInput v-model="keyword" placeholder="搜索房间名称或编码" />
        <AppButton v-if="canManage" icon="plus" variant="primary" @click="openCreate">
          新建房间
        </AppButton>
      </template>
    </PageHeader>

    <div class="mt-4 flex items-center justify-between gap-3">
      <div class="flex items-center gap-2">
        <div class="w-[200px]">
          <select v-model="areaFilter" class="select" aria-label="按区域筛选房间">
            <option value="">全部区域</option>
            <option v-for="area in areas" :key="area.id" :value="area.id">
              {{ area.name }}
            </option>
          </select>
        </div>
        <label class="flex items-center gap-2 text-[11.5px] text-muted">
          <input
            v-model="includeInactive"
            type="checkbox"
            class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
          />
          显示已停用
        </label>
        <AppButton v-if="hasFilter" size="sm" variant="ghost" @click="resetFilters">
          重置筛选
        </AppButton>
      </div>
      <span class="text-[11px] text-faint">共 {{ total }} 个房间</span>
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
            <th class="table-th pl-5">房间</th>
            <th class="table-th">区域</th>
            <th class="table-th">容量</th>
            <th class="table-th">设备槽位</th>
            <th class="table-th">在线状态</th>
            <th class="table-th">更新时间</th>
            <th class="table-th pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="room in items"
            :key="room.id"
            class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            :class="room.is_active ? '' : 'opacity-70'"
            @click="openDetail(room)"
          >
            <td class="table-td pl-5">
              <div class="flex items-center gap-2">
                <span class="text-[12px] font-medium text-ink">{{ room.name }}</span>
                <StateBadge v-if="!room.is_active" label="已停用" tone="slate" />
              </div>
              <div class="mt-0.5 font-mono text-[10.5px] text-faint">{{ room.code }}</div>
            </td>
            <td class="table-td">{{ room.area_name }}</td>
            <td class="table-td font-mono text-[11px]">{{ room.capacity }}</td>
            <td class="table-td">
              <span class="font-mono text-[11px]">{{ slotLabel(room) }}</span>
            </td>
            <td class="table-td">
              <StateBadge
                :label="onlineBadge(room).label"
                :tone="onlineBadge(room).tone"
                :pulse="room.device?.online === 'online'"
              />
            </td>
            <td class="table-td text-[11px] text-faint">
              {{ formatRelative(room.updated_at) }}
            </td>
            <td class="table-td pr-5" @click.stop>
              <div class="flex items-center justify-end gap-1">
                <IconButton
                  v-if="canManage"
                  icon="pencil"
                  label="编辑房间"
                  size="sm"
                  @click="openEdit(room)"
                />
                <IconButton
                  v-if="canManage"
                  icon="trash"
                  label="删除房间"
                  size="sm"
                  @click="askDelete(room)"
                />
                <IconButton
                  icon="chevron-right"
                  label="查看房间详情"
                  size="sm"
                  @click="openDetail(room)"
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
        :title="hasFilter ? '没有匹配的房间' : '还没有房间'"
        :description="
          hasFilter
            ? '试试调整区域、关键词或勾选显示已停用后再试。'
            : '房间是排课与设备绑定的最小单位，创建后可以继续绑定设备、配置课表。'
        "
        icon="door"
      >
        <template v-if="canManage && !hasFilter" #action>
          <AppButton icon="plus" variant="primary" @click="openCreate">新建房间</AppButton>
        </template>
      </EmptyState>

      <EmptyState v-else title="加载房间失败" :description="error" icon="alert">
        <template #action>
          <AppButton icon="refresh" @click="load()">重试</AppButton>
        </template>
      </EmptyState>

      <PaginationBar
        v-if="!loading && items.length"
        :page="page"
        :total-pages="totalPages"
        :total="total"
        label="个房间"
        @change="(next) => (page = next)"
      />
    </section>

    <RoomFormModal
      :open="formOpen"
      :room="editing"
      :areas="areas"
      @close="formOpen = false"
      @saved="onSaved"
    />

    <ConfirmDialog
      :open="deleteTarget !== null"
      title="删除房间"
      :description="`确认删除房间「${deleteTarget?.name ?? ''}」吗？`"
      detail="删除房间会级联删除该房间的课表与触发规则，且不可恢复。"
      confirm-label="删除房间"
      danger
      :loading="deleting"
      @close="deleteTarget = null"
      @confirm="confirmDelete"
    />
  </div>
</template>
