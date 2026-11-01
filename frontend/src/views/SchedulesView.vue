<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import { api, isForbidden } from '@/api'
import type { Room, Schedule } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import ScheduleFormModal from '@/features/schedules/ScheduleFormModal.vue'
import type { ScheduleFormValue } from '@/features/schedules/scheduleForm'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatRelative } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const PAGE_SIZE = 20

const auth = useAuthStore()
const toast = useToastStore()
const router = useRouter()

const items = ref<Schedule[]>([])
const total = ref(0)
const page = ref(1)
const loading = ref(false)
const error = ref('')
const rooms = ref<Room[]>([])
const roomFilter = ref('')

const formOpen = ref(false)
const creating = ref(false)
const deleteTarget = ref<Schedule | null>(null)
const deleting = ref(false)

const canManage = computed(() => auth.can(PERMISSIONS.scheduleManage))
const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const sortedRooms = computed(() =>
  [...rooms.value].sort((left, right) => left.name.localeCompare(right.name, 'zh-CN')),
)

function actionError(caught: unknown, fallback: string): string {
  return isForbidden(caught) ? '缺少课表管理权限' : errorMessage(caught, fallback)
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const result = await api.listSchedules({
      room_id: roomFilter.value || undefined,
      page: page.value,
      page_size: PAGE_SIZE,
    })
    items.value = result.items ?? []
    total.value = result.total ?? 0
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载课表列表失败')
    items.value = []
    total.value = 0
  } finally {
    loading.value = false
  }
}

async function loadRooms() {
  try {
    const result = await api.listRooms({ page_size: 100 })
    rooms.value = result.items ?? []
  } catch {
    // 房间下拉加载失败不阻塞列表，仅影响筛选/新建时的房间选项。
    rooms.value = []
  }
}

function applyFilter() {
  page.value = 1
  void load()
}

function loadPage(next: number) {
  page.value = next
  void load()
}

function openCreate() {
  formOpen.value = true
}

async function submitCreate(value: ScheduleFormValue) {
  creating.value = true
  try {
    const created = await api.createSchedule({
      room_id: value.room_id,
      name: value.name,
      start_date: value.start_date,
      end_date: value.end_date,
      is_active: value.is_active,
    })
    formOpen.value = false
    toast.success('课表已创建', created.name)
    await router.push({ name: 'schedule-detail', params: { id: created.id } })
  } catch (caught) {
    toast.error('创建课表失败', actionError(caught, '请稍后重试'))
  } finally {
    creating.value = false
  }
}

function requestDelete(schedule: Schedule) {
  deleteTarget.value = schedule
}

async function confirmDelete() {
  const target = deleteTarget.value
  if (!target) return
  deleting.value = true
  try {
    await api.deleteSchedule(target.id)
    deleteTarget.value = null
    toast.success('课表已删除', target.name)
    if (items.value.length === 1 && page.value > 1) page.value -= 1
    await load()
  } catch (caught) {
    toast.error('删除课表失败', actionError(caught, '请稍后重试'))
  } finally {
    deleting.value = false
  }
}

function openDetail(schedule: Schedule) {
  void router.push({ name: 'schedule-detail', params: { id: schedule.id } })
}

onMounted(() => {
  void load()
  void loadRooms()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="课表" description="管理的周模板、单日覆盖与排课实例；课表归属于具体房间。">
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新课表列表"
          variant="outline"
          :loading="loading"
          :disabled="loading"
          @click="load()"
        />
        <AppButton v-if="canManage" icon="plus" variant="primary" @click="openCreate">
          新建课表
        </AppButton>
      </template>
    </PageHeader>

    <div
      v-if="error"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <section class="mt-4 overflow-hidden rounded-xl border border-line bg-surface">
      <div class="flex items-center justify-between gap-3 border-b border-line px-5 py-3">
        <div class="flex items-center gap-2.5">
          <label class="text-[11.5px] text-muted" for="schedule-room-filter">房间</label>
          <select
            id="schedule-room-filter"
            v-model="roomFilter"
            class="select w-[220px]"
            aria-label="按房间筛选课表"
            @change="applyFilter"
          >
            <option value="">全部房间</option>
            <option v-for="room in sortedRooms" :key="room.id" :value="room.id">
              {{ room.name }}{{ room.is_active ? '' : '（已停用）' }}
            </option>
          </select>
        </div>
        <p class="text-[10.5px] text-faint">
          共 {{ total }} 张课表 · 第 {{ page }} / {{ totalPages }} 页
        </p>
      </div>

      <table v-if="loading || items.length" class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">课表名</th>
            <th class="table-th">房间</th>
            <th class="table-th">起止日期</th>
            <th class="table-th">时区</th>
            <th class="table-th">状态</th>
            <th class="table-th">更新时间</th>
            <th class="table-th pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <template v-if="loading">
            <tr v-for="index in 5" :key="`skeleton-${index}`" class="border-b border-line/70">
              <td colspan="7" class="px-5 py-2.5">
                <div class="h-6 animate-pulse rounded bg-line/60" />
              </td>
            </tr>
          </template>
          <template v-else>
            <tr
              v-for="schedule in items"
              :key="schedule.id"
              class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
              @click="openDetail(schedule)"
            >
              <td class="table-td pl-5">
                <div class="flex items-center gap-2">
                  <span class="text-[12.5px] font-medium text-ink">{{ schedule.name }}</span>
                  <span class="font-mono text-[10px] text-faint">{{
                    schedule.id.slice(0, 8)
                  }}</span>
                </div>
              </td>
              <td class="table-td">{{ schedule.room_name }}</td>
              <td class="table-td font-mono text-[11px]">
                {{ schedule.start_date }} ~ {{ schedule.end_date }}
              </td>
              <td class="table-td font-mono text-[11px]">{{ schedule.timezone }}</td>
              <td class="table-td">
                <StateBadge
                  :label="schedule.is_active ? '启用' : '停用'"
                  :tone="schedule.is_active ? 'emerald' : 'slate'"
                />
              </td>
              <td class="table-td text-[10.5px] text-faint">
                {{ formatRelative(schedule.updated_at) }}
              </td>
              <td class="table-td pr-5 text-right">
                <div class="flex items-center justify-end gap-1" @click.stop>
                  <IconButton
                    icon="arrow-up-right"
                    label="查看课表详情"
                    size="sm"
                    @click="openDetail(schedule)"
                  />
                  <IconButton
                    v-if="canManage"
                    icon="trash"
                    label="删除课表"
                    size="sm"
                    :disabled="deleting"
                    @click="requestDelete(schedule)"
                  />
                </div>
              </td>
            </tr>
          </template>
        </tbody>
      </table>

      <EmptyState
        v-else-if="!error"
        title="还没有课表"
        description="课表按房间维护周模板，保存后会自动生成排课实例并触发事件。"
        icon="calendar"
      >
        <template v-if="canManage" #action>
          <AppButton size="sm" icon="plus" variant="primary" @click="openCreate"
            >新建课表</AppButton
          >
        </template>
      </EmptyState>

      <PaginationBar
        v-if="!loading && total > 0"
        :page="page"
        :total-pages="totalPages"
        :total="total"
        label="张课表"
        @change="loadPage"
      />
    </section>

    <p class="mt-3 flex items-center gap-1.5 text-[10.5px] text-faint">
      <AppIcon name="info" :size="12" />
      课表时区随房间配置；点击任意行进入详情，维护周模板、单日覆盖与排课实例。
    </p>

    <ScheduleFormModal
      :open="formOpen"
      mode="create"
      :rooms="rooms"
      :saving="creating"
      @close="formOpen = false"
      @submit="submitCreate"
    />

    <ConfirmDialog
      :open="!!deleteTarget"
      title="删除课表"
      :description="`删除课表「${deleteTarget?.name ?? ''}」及其全部排课实例。`"
      detail="该操作不可撤销，相关单日覆盖与排课实例会一并删除。"
      confirm-label="删除课表"
      danger
      :loading="deleting"
      @confirm="confirmDelete"
      @close="deleteTarget = null"
    />
  </div>
</template>
