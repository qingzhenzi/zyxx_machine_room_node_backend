<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'

import { api } from '@/api'
import type { ImageArtifact, Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
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
import { batchVmNamePreview, isValidVmName, VM_CREATE_LIMITS } from '@/features/machines/vmCommand'
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

// ---------------------------------------------------------------------------
// 下发环境配置（阶段 1.5，教师批量）：选房间 + 底包 + 规格 → 逐槽位下发 create。
// ---------------------------------------------------------------------------

interface EnvTargetRow {
  device_slot: string
  serial: string | null
  request_id: string | null
  status: string
  error: string | null
}

const canVmControl = computed(() => auth.can(PERMISSIONS.vmControl))
const envOpen = ref(false)
const envError = ref('')
const envSubmitting = ref(false)
const envImages = ref<ImageArtifact[]>([])
const envImagesLoading = ref(false)
const envTargets = ref<EnvTargetRow[]>([])
const envForm = ref({
  roomId: '',
  namePrefix: 'lab',
  baseVolume: '',
  vcpus: 2,
  memoryMiB: 2048,
  diskGiB: 20,
  autoStart: true,
})

const envBaseVolumeOptions = computed(() =>
  envImages.value.map((image) => ({
    value: `${image.name}-${image.version}`,
    label: `${image.name}-${image.version}`,
    description: `${image.platform} · ${image.format}`,
  })),
)

function envStatusMeta(status: string): {
  label: string
  tone: 'emerald' | 'amber' | 'rose' | 'slate'
} {
  switch (status) {
    case 'ok':
      return { label: '已创建', tone: 'emerald' }
    case 'error':
      return { label: '失败', tone: 'rose' }
    case 'timeout':
      return { label: '超时', tone: 'amber' }
    case 'skipped':
      return { label: '未下发', tone: 'slate' }
    default:
      return { label: '等待中', tone: 'amber' }
  }
}

async function loadEnvImages() {
  envImagesLoading.value = true
  try {
    const page = await api.listImages({
      kind: 'vm_disk',
      status: 'published',
      page: 1,
      page_size: 50,
    })
    envImages.value = page.items ?? []
  } catch {
    envImages.value = []
  } finally {
    envImagesLoading.value = false
  }
}

function openEnvModal() {
  envError.value = ''
  envTargets.value = []
  envForm.value = {
    roomId: rooms.value[0]?.id ?? '',
    namePrefix: 'lab',
    baseVolume: envBaseVolumeOptions.value[0]?.value ?? '',
    vcpus: 2,
    memoryMiB: 2048,
    diskGiB: 20,
    autoStart: true,
  }
  envOpen.value = true
  if (!envImages.value.length) void loadEnvImages()
}

function validateEnvForm(): string {
  const form = envForm.value
  if (!form.roomId) return '请选择房间'
  if (!form.namePrefix.trim()) return '请填写名称前缀'
  if (!isValidVmName(form.namePrefix.trim())) {
    return '名称前缀只允许字母、数字、点、下划线、短横线（1-32 字符）'
  }
  if (form.namePrefix.trim().length > 32) return '名称前缀不能超过 32 个字符'
  if (!form.baseVolume.trim()) return '请选择系统底包（已发布 vm_disk 镜像）'
  if (
    !Number.isInteger(form.vcpus) ||
    form.vcpus < VM_CREATE_LIMITS.vcpus.min ||
    form.vcpus > VM_CREATE_LIMITS.vcpus.max
  ) {
    return `vCPU 需在 ${VM_CREATE_LIMITS.vcpus.min}..=${VM_CREATE_LIMITS.vcpus.max} 之间`
  }
  if (
    !Number.isInteger(form.memoryMiB) ||
    form.memoryMiB < VM_CREATE_LIMITS.memoryMiB.min ||
    form.memoryMiB > VM_CREATE_LIMITS.memoryMiB.max
  ) {
    return `内存需在 ${VM_CREATE_LIMITS.memoryMiB.min}..=${VM_CREATE_LIMITS.memoryMiB.max} MiB 之间`
  }
  if (
    !Number.isInteger(form.diskGiB) ||
    form.diskGiB < VM_CREATE_LIMITS.diskGiB.min ||
    form.diskGiB > VM_CREATE_LIMITS.diskGiB.max
  ) {
    return `磁盘需在 ${VM_CREATE_LIMITS.diskGiB.min}..=${VM_CREATE_LIMITS.diskGiB.max} GiB 之间`
  }
  return ''
}

async function submitEnv() {
  const error = validateEnvForm()
  envError.value = error
  if (error) return
  const form = envForm.value
  envSubmitting.value = true
  try {
    const reply = await api.batchVmEnvironments(form.roomId, {
      name_prefix: form.namePrefix.trim(),
      base_volume: form.baseVolume.trim(),
      vcpus: form.vcpus,
      memory_mib: form.memoryMiB,
      disk_gb: form.diskGiB,
      auto_start: form.autoStart,
    })
    envTargets.value = reply.targets.map((target) => ({
      device_slot: target.device_slot,
      serial: target.serial,
      request_id: target.request_id,
      status: target.request_id ? 'pending' : 'skipped',
      error: null,
    }))
    void pollEnvTargets(30)
  } catch (caught) {
    envError.value = errorMessage(caught, '批量下发失败')
  } finally {
    envSubmitting.value = false
  }
}

async function pollEnvTargets(remaining: number) {
  const pending = envTargets.value.filter(
    (target) => target.request_id && target.serial && target.status === 'pending',
  )
  if (!pending.length || remaining <= 0) return
  await Promise.allSettled(
    pending.map(async (target) => {
      const reply = await api.vmCommandResult(target.serial as string, target.request_id as string)
      const next = envTargets.value.map((row) =>
        row.request_id === target.request_id
          ? { ...row, status: reply.status, error: reply.error ?? null }
          : row,
      )
      envTargets.value = next
    }),
  )
  if (envTargets.value.some((target) => target.status === 'pending')) {
    setTimeout(() => void pollEnvTargets(remaining - 1), 1000)
  }
}

function namePreview(slot: string): string {
  return batchVmNamePreview(envForm.value.namePrefix || 'lab', slot)
}

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

      <PageSection
        class="col-span-8"
        title="下发环境配置"
        description="为一个房间的主设备 + 子机批量创建实验虚拟机（qcow2 差分盘，底包只读共享）"
      >
        <div class="flex flex-wrap items-center justify-between gap-3 px-5 py-4">
          <p class="max-w-[520px] text-[11.5px] leading-5 text-muted">
            选择房间与系统底包后，按「主设备 + 子机」逐槽位下发 <code>create</code>， 每台 VM 命名
            <code>&lt;前缀&gt;-&lt;槽位尾号&gt;</code>；各槽位生成独立 request_id 并可轮询状态。
          </p>
          <div class="flex items-center gap-2">
            <AppButton v-if="canVmControl" icon="layers" @click="openEnvModal">
              选择房间并下发
            </AppButton>
            <span v-else class="text-[10.5px] text-faint">需要 vm:control 权限</span>
          </div>
        </div>
      </PageSection>

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

    <AppModal
      :open="envOpen"
      title="下发环境配置"
      description="房间内每个设备槽位各下发一条 create（vm:control）；命名 &lt;前缀&gt;-&lt;槽位尾号&gt;"
      size="lg"
      @close="envOpen = false"
    >
      <div class="grid grid-cols-2 gap-3">
        <label class="flex flex-col gap-1 text-[11px] text-muted">
          房间
          <select
            v-model="envForm.roomId"
            class="h-9 rounded-md border border-line bg-surface px-2 text-[12px] text-ink outline-none focus:border-accent"
          >
            <option v-for="room in rooms" :key="room.id" :value="room.id">
              {{ room.name }}（{{ room.code }}）
            </option>
          </select>
        </label>
        <label class="flex flex-col gap-1 text-[11px] text-muted">
          系统底包（已发布 vm_disk 镜像）
          <select
            v-model="envForm.baseVolume"
            :disabled="envImagesLoading"
            class="h-9 rounded-md border border-line bg-surface px-2 text-[12px] text-ink outline-none focus:border-accent"
          >
            <option value="">请选择底包</option>
            <option
              v-for="option in envBaseVolumeOptions"
              :key="option.value"
              :value="option.value"
            >
              {{ option.label }}（{{ option.description }}）
            </option>
          </select>
        </label>
        <label class="flex flex-col gap-1 text-[11px] text-muted">
          名称前缀
          <input
            v-model="envForm.namePrefix"
            placeholder="lab"
            class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
          />
        </label>
        <label class="flex flex-col gap-1 text-[11px] text-muted">
          vCPU（1-64）
          <input
            v-model.number="envForm.vcpus"
            type="number"
            min="1"
            max="64"
            class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
          />
        </label>
        <label class="flex flex-col gap-1 text-[11px] text-muted">
          内存 MiB（256-262144）
          <input
            v-model.number="envForm.memoryMiB"
            type="number"
            min="256"
            max="262144"
            class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
          />
        </label>
        <label class="flex flex-col gap-1 text-[11px] text-muted">
          磁盘 GiB（1-2048）
          <input
            v-model.number="envForm.diskGiB"
            type="number"
            min="1"
            max="2048"
            class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
          />
        </label>
        <label class="col-span-2 flex items-center gap-2 text-[11.5px] text-muted">
          <input
            v-model="envForm.autoStart"
            type="checkbox"
            class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
          />
          创建后自动启动
        </label>
      </div>

      <p v-if="envError" class="mt-2 text-[11px] text-rose-600">{{ envError }}</p>

      <ul
        v-if="envTargets.length"
        class="mt-3 divide-y divide-line/70 rounded-md border border-line"
      >
        <li
          v-for="target in envTargets"
          :key="target.device_slot"
          class="flex flex-wrap items-center justify-between gap-2 px-3 py-2"
        >
          <div class="min-w-0">
            <p class="truncate text-[11px] text-ink">
              {{ namePreview(target.device_slot) }}
              <span class="text-[10px] text-faint">· {{ target.device_slot }}</span>
            </p>
            <p class="mt-0.5 break-all font-mono text-[10px] text-faint">
              {{ target.request_id ?? '未下发（槽位无存活设备）' }}
            </p>
          </div>
          <div class="flex items-center gap-1.5">
            <StateBadge
              :label="envStatusMeta(target.status).label"
              :tone="envStatusMeta(target.status).tone"
            />
            <span v-if="target.error" class="max-w-[240px] truncate text-[10px] text-rose-600">
              {{ target.error }}
            </span>
          </div>
        </li>
      </ul>

      <template #footer>
        <AppButton :disabled="envSubmitting" @click="envOpen = false">关闭</AppButton>
        <AppButton
          :disabled="envSubmitting || !envTargets.length"
          icon="refresh"
          @click="pollEnvTargets(30)"
        >
          刷新状态
        </AppButton>
        <AppButton variant="primary" :loading="envSubmitting" @click="submitEnv">
          下发环境
        </AppButton>
      </template>
    </AppModal>
  </div>
</template>
