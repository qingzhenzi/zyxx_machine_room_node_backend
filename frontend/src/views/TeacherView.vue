<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { api } from '@/api'
import type { Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import type { BoardPayload } from '@/features/scheduleBoard/boardTypes'
import {
  blocksOfDate,
  expandBoardBlocks,
  isoWeekday,
  weekdayName,
} from '@/features/scheduleBoard/boardTypes'
import { useAuthStore } from '@/stores/auth'
import { errorMessage } from '@/utils/errors'
import { PERMISSIONS } from '@/utils/permissions'

/**
 * 教师工作台（阶段 1）：
 * - 今日课程：复用排课工作台聚合读模型（schedule-board），展开今天的课块；
 * - 我的房间：房间列表快捷入口；
 * - 快捷操作卡：镜像 / 工单 / 设备唤醒 / VM 控制；
 * - 实验环境模板：占位（明确「即将上线」）。
 *
 * 数据读取失败只显示友好空态，不阻塞其它区块。
 */

const auth = useAuthStore()

const today = new Date().toISOString().slice(0, 10)
const todayLabel = `${today} ${weekdayName(isoWeekday(today))}`

const board = ref<BoardPayload | null>(null)
const boardLoading = ref(false)
const boardError = ref('')

const rooms = ref<Room[]>([])
const roomsLoading = ref(false)
const roomsError = ref('')

const refreshLoading = ref(false)

const todayBlocks = computed(() => {
  if (!board.value) return []
  const blocks = expandBoardBlocks(board.value, today, today)
  return blocksOfDate(blocks, today).sort((a, b) => a.start_time.localeCompare(b.start_time))
})

const quickCards = computed(() =>
  [
    {
      title: '镜像管理',
      description: '制作、发布与分配实验镜像',
      to: '/images',
      icon: 'package',
      permission: PERMISSIONS.imageRead,
    },
    {
      title: '工单',
      description: '报修设备、跟踪处理进度',
      to: '/tickets',
      icon: 'ticket',
      permission: PERMISSIONS.ticketRead,
    },
    {
      title: '设备唤醒',
      description: '机器详情 → 网络唤醒',
      to: '/machines',
      icon: 'zap',
      permission: PERMISSIONS.deviceRead,
    },
    {
      title: 'VM 控制',
      description: '机器详情 → 虚拟机面板',
      to: '/machines',
      icon: 'server',
      permission: PERMISSIONS.vmControl,
    },
  ].filter((card) => auth.can(card.permission)),
)

const experimentTemplates = [
  { name: 'Linux 基础实验', description: '命令行、进程与文件系统练习', icon: 'terminal' },
  { name: '网络协议实验', description: '抓包、路由与 DNS 实验环境', icon: 'wifi' },
  { name: '容器与虚拟化', description: 'Docker / LXC / KVM 实验模板', icon: 'layers' },
] as const

async function loadBoard() {
  boardLoading.value = true
  boardError.value = ''
  try {
    board.value = await api.scheduleBoard({ from: today, to: today })
  } catch (caught) {
    boardError.value = errorMessage(caught, '今日课程加载失败')
  } finally {
    boardLoading.value = false
  }
}

async function loadRooms() {
  roomsLoading.value = true
  roomsError.value = ''
  try {
    const page = await api.listRooms({ page: 1, page_size: 12 })
    rooms.value = page.items ?? []
  } catch (caught) {
    roomsError.value = errorMessage(caught, '房间列表加载失败')
  } finally {
    roomsLoading.value = false
  }
}

async function refreshAll() {
  refreshLoading.value = true
  await Promise.allSettled([loadBoard(), loadRooms()])
  refreshLoading.value = false
}

onMounted(() => void refreshAll())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="教师工作台" :description="`${todayLabel} · 今日课程与教学快捷入口`">
      <template #actions>
        <AppButton icon="refresh" :loading="refreshLoading" @click="refreshAll">刷新</AppButton>
      </template>
    </PageHeader>

    <div class="mt-4 grid grid-cols-12 items-start gap-4">
      <!-- 今日课程 -->
      <PageSection
        class="col-span-8"
        title="今日课程"
        description="来自排课工作台（schedule-board）的今日课块"
      >
        <div v-if="boardError" class="px-5 py-4">
          <EmptyState title="今日课程加载失败" :description="boardError" icon="calendar" />
        </div>
        <div v-else-if="boardLoading" class="px-5 py-6 text-[11.5px] text-muted">
          正在加载今日课程…
        </div>
        <ul v-else-if="todayBlocks.length" class="divide-y divide-line/70">
          <li
            v-for="block in todayBlocks"
            :key="block.key"
            class="flex flex-wrap items-center justify-between gap-3 px-5 py-3"
          >
            <div class="flex min-w-0 items-center gap-3">
              <div class="w-24 shrink-0 font-mono text-[11px] text-ink-soft">
                {{ block.start_time }}–{{ block.end_time }}
              </div>
              <div class="min-w-0">
                <p class="truncate text-[12px] font-medium text-ink">{{ block.course_name }}</p>
                <p class="mt-0.5 truncate text-[10.5px] text-faint">
                  {{ block.room_name }}（{{ block.room_code }}）
                  <template v-if="block.teacher"> · {{ block.teacher }}</template>
                </p>
              </div>
            </div>
            <StateBadge v-if="block.is_override" label="调课" tone="sky" />
          </li>
        </ul>
        <EmptyState
          v-else
          title="今天没有课程"
          description="排课工作台里今天没有展开出课块；可到「课表」页面检查周模板与生效区间。"
          icon="calendar"
        />
      </PageSection>

      <!-- 我的房间 -->
      <PageSection
        class="col-span-4"
        title="我的房间"
        description="快捷进入房间总览（设备/课表/镜像）"
      >
        <div v-if="roomsError" class="px-5 py-4">
          <EmptyState title="房间加载失败" :description="roomsError" icon="door" />
        </div>
        <div v-else-if="roomsLoading" class="px-5 py-6 text-[11.5px] text-muted">正在加载房间…</div>
        <ul v-else-if="rooms.length" class="divide-y divide-line/70">
          <li v-for="room in rooms" :key="room.id">
            <RouterLink
              :to="{ name: 'room-detail', params: { id: room.id } }"
              class="flex items-center justify-between gap-2 px-5 py-2.5 transition-colors hover:bg-canvas/50"
            >
              <div class="min-w-0">
                <p class="truncate text-[11.5px] text-ink">{{ room.name }}</p>
                <p class="mt-0.5 truncate text-[10.5px] text-faint">
                  {{ room.area_name }} · {{ room.code }} · {{ room.capacity }} 座
                </p>
              </div>
              <AppIcon name="chevron-right" :size="13" class="text-faint" />
            </RouterLink>
          </li>
        </ul>
        <EmptyState
          v-else
          title="暂无房间"
          description="还没有房间数据，或当前账号看不到房间。"
          icon="door"
        />
      </PageSection>

      <!-- 快捷操作 -->
      <div class="col-span-8">
        <div class="grid grid-cols-2 gap-3">
          <RouterLink
            v-for="card in quickCards"
            :key="card.title"
            :to="card.to"
            class="group flex items-start gap-3 rounded-xl border border-line bg-surface px-4 py-3.5 transition-colors hover:border-line-strong"
          >
            <span
              class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-canvas text-ink-soft transition-colors group-hover:text-accent"
            >
              <AppIcon :name="card.icon" :size="16" />
            </span>
            <span class="min-w-0">
              <span class="block text-[12px] font-medium text-ink">{{ card.title }}</span>
              <span class="mt-0.5 block text-[10.5px] text-muted">{{ card.description }}</span>
            </span>
          </RouterLink>
        </div>
      </div>

      <!-- 实验模板占位 -->
      <PageSection
        class="col-span-4"
        title="实验环境模板"
        description="一键创建课程实验环境（阶段 2）"
      >
        <ul class="divide-y divide-line/70">
          <li
            v-for="template in experimentTemplates"
            :key="template.name"
            class="flex items-center justify-between gap-2 px-5 py-3"
          >
            <div class="flex min-w-0 items-center gap-2.5">
              <AppIcon :name="template.icon" :size="14" class="text-faint" />
              <div class="min-w-0">
                <p class="truncate text-[11.5px] text-ink">{{ template.name }}</p>
                <p class="mt-0.5 truncate text-[10.5px] text-faint">{{ template.description }}</p>
              </div>
            </div>
            <StateBadge label="即将上线" tone="slate" />
          </li>
        </ul>
      </PageSection>
    </div>
  </div>
</template>
