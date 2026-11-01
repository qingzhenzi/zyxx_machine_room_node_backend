<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import { api } from '@/api'
import type { SearchResults } from '@/api/types'
import AppIcon from '@/components/ui/AppIcon.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import { useAuthStore } from '@/stores/auth'
import { useUiStore } from '@/stores/ui'
import { PERMISSIONS } from '@/utils/permissions'

import CommandPaletteResults from './CommandPaletteResults.vue'
import type { PaletteEntry } from './types'

const ui = useUiStore()
const router = useRouter()
const auth = useAuthStore()

const query = ref('')
const results = ref<SearchResults | null>(null)
const loading = ref(false)
const activeIndex = ref(0)
const inputRef = ref<HTMLInputElement | null>(null)

let searchToken = 0

const { run: scheduleSearch, cancel: cancelSearch } = useDebounceFn((keyword: string) => {
  void runSearch(keyword)
}, 200)

function closePalette() {
  ui.commandPaletteOpen = false
}

function navigate(location: Parameters<typeof router.push>[0]) {
  closePalette()
  void router.push(location)
}

const quickActions = computed<PaletteEntry[]>(() => {
  const actions: (PaletteEntry & { permission?: string })[] = [
    {
      key: 'nav-overview',
      title: '总览',
      subtitle: '房间、设备、课表与系统状态',
      icon: 'overview',
      section: '跳转',
      action: () => navigate({ name: 'overview' }),
    },
    {
      key: 'nav-areas',
      title: '区域',
      subtitle: '楼栋 / 楼层 / 片区与 webhook',
      icon: 'layers',
      section: '跳转',
      permission: PERMISSIONS.areaRead,
      action: () => navigate({ name: 'areas' }),
    },
    {
      key: 'nav-rooms',
      title: '房间',
      subtitle: '房间、设备槽位与房间级 webhook',
      icon: 'door',
      section: '跳转',
      permission: PERMISSIONS.roomRead,
      action: () => navigate({ name: 'rooms' }),
    },
    {
      key: 'nav-devices',
      title: '设备',
      subtitle: '注册状态、在线状态与房间绑定',
      icon: 'monitor',
      section: '跳转',
      permission: PERMISSIONS.deviceRead,
      action: () => navigate({ name: 'devices' }),
    },
    {
      key: 'nav-schedules',
      title: '课表',
      subtitle: '周模板、单日覆盖与排课实例',
      icon: 'calendar',
      section: '跳转',
      permission: PERMISSIONS.scheduleRead,
      action: () => navigate({ name: 'schedules' }),
    },
    {
      key: 'nav-events',
      title: '触发事件',
      subtitle: '事件记录、投递明细与重放',
      icon: 'zap',
      section: '跳转',
      permission: PERMISSIONS.scheduleEventRead,
      action: () => navigate({ name: 'events' }),
    },
    {
      key: 'nav-images',
      title: '镜像',
      subtitle: 'manifest、验证、发布与房间分配',
      icon: 'package',
      section: '跳转',
      permission: PERMISSIONS.imageRead,
      action: () => navigate({ name: 'images' }),
    },
    {
      key: 'nav-storage',
      title: '存储',
      subtitle: '对象预签名、分发任务与收集登记',
      icon: 'database',
      section: '跳转',
      permission: PERMISSIONS.storageRead,
      action: () => navigate({ name: 'storage' }),
    },
    {
      key: 'nav-agent-configs',
      title: '节点配置',
      subtitle: 'global / area / room 生效策略',
      icon: 'sliders',
      section: '跳转',
      permission: PERMISSIONS.roomRead,
      action: () => navigate({ name: 'agent-configs' }),
    },
    {
      key: 'nav-users',
      title: '用户',
      subtitle: '账号、状态与角色分配',
      icon: 'users',
      section: '跳转',
      permission: PERMISSIONS.userManage,
      action: () => navigate({ name: 'users' }),
    },
    {
      key: 'nav-roles',
      title: '角色',
      subtitle: '自定义角色与权限点',
      icon: 'shield',
      section: '跳转',
      permission: PERMISSIONS.roleManage,
      action: () => navigate({ name: 'roles' }),
    },
    {
      key: 'nav-audit',
      title: '审计日志',
      subtitle: '管理操作与资源变更记录',
      icon: 'scroll',
      section: '跳转',
      permission: PERMISSIONS.auditRead,
      action: () => navigate({ name: 'audit' }),
    },
    {
      key: 'nav-announcements',
      title: '公告',
      subtitle: '草稿、发布、归档与置顶',
      icon: 'megaphone',
      section: '跳转',
      permission: PERMISSIONS.announcementRead,
      action: () => navigate({ name: 'announcements' }),
    },
    {
      key: 'nav-account',
      title: '账户',
      subtitle: '个人资料、设备会话与邮箱验证',
      icon: 'user',
      section: '跳转',
      action: () => navigate({ name: 'account' }),
    },
  ]

  return actions
    .filter((item) => !item.permission || auth.can(item.permission))
    .map((item) => ({
      key: item.key,
      title: item.title,
      subtitle: item.subtitle,
      icon: item.icon,
      section: item.section,
      action: item.action,
    }))
})

const entries = computed<PaletteEntry[]>(() => {
  const keyword = query.value.trim()
  if (!keyword) return quickActions.value

  const data = results.value
  if (!data) return []

  return [
    ...data.rooms.map<PaletteEntry>((room) => ({
      key: `room-${room.id}`,
      title: room.name,
      subtitle: `${room.code} · ${room.area_name}`,
      icon: 'door',
      section: '房间',
      action: () => navigate({ name: 'room-detail', params: { id: room.id } }),
    })),
    ...data.devices.map<PaletteEntry>((device) => ({
      key: `device-${device.serial}`,
      title: device.room_name ?? device.device_slot,
      subtitle: `${device.device_slot} · ${device.serial.slice(0, 12)}`,
      icon: 'monitor',
      section: '设备',
      action: () => navigate({ name: 'device-detail', params: { serial: device.serial } }),
    })),
    ...data.images.map<PaletteEntry>((image) => ({
      key: `image-${image.id}`,
      title: `${image.name}:${image.version}`,
      subtitle: `${image.kind} · ${image.platform} · ${image.status}`,
      icon: 'package',
      section: '镜像',
      action: () => navigate({ name: 'image-detail', params: { id: image.id } }),
    })),
    ...data.schedules.map<PaletteEntry>((schedule) => ({
      key: `schedule-${schedule.id}`,
      title: schedule.name,
      subtitle: `${schedule.room_name} · ${schedule.start_date} ~ ${schedule.end_date}`,
      icon: 'calendar',
      section: '课表',
      action: () => navigate({ name: 'schedule-detail', params: { id: schedule.id } }),
    })),
    ...data.announcements.map<PaletteEntry>((announcement) => ({
      key: `announcement-${announcement.id}`,
      title: announcement.title,
      subtitle: announcement.status,
      icon: 'megaphone',
      section: '公告',
      action: () => navigate({ name: 'announcements', query: { q: announcement.title } }),
    })),
  ]
})

watch(query, (value) => {
  const keyword = value.trim()
  if (!keyword) {
    cancelSearch()
    results.value = null
    loading.value = false
    return
  }

  loading.value = true
  scheduleSearch(keyword)
})

watch(entries, () => {
  activeIndex.value = 0
})

watch(
  () => ui.commandPaletteOpen,
  async (open) => {
    if (!open) return
    query.value = ''
    results.value = null
    await nextTick()
    inputRef.value?.focus()
  },
)

async function runSearch(keyword: string) {
  const token = ++searchToken
  try {
    const data = await api.search(keyword)
    if (token === searchToken) results.value = data
  } catch {
    if (token === searchToken) {
      results.value = { rooms: [], devices: [], images: [], schedules: [], announcements: [] }
    }
  } finally {
    if (token === searchToken) loading.value = false
  }
}

function onKeydown(event: KeyboardEvent) {
  if (!ui.commandPaletteOpen) return

  if (event.key === 'Escape') {
    event.preventDefault()
    closePalette()
    return
  }

  if (event.key === 'ArrowDown') {
    event.preventDefault()
    if (!entries.value.length) return
    activeIndex.value = (activeIndex.value + 1) % entries.value.length
    return
  }

  if (event.key === 'ArrowUp') {
    event.preventDefault()
    if (!entries.value.length) return
    activeIndex.value = (activeIndex.value - 1 + entries.value.length) % entries.value.length
    return
  }

  if (event.key === 'Enter') {
    event.preventDefault()
    const entry = entries.value[activeIndex.value]
    entry?.action()
  }
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <Teleport to="body">
    <Transition name="palette">
      <div
        v-if="ui.commandPaletteOpen"
        class="fixed inset-0 z-[60] flex items-start justify-center bg-ink/30 px-6 pt-[12vh] backdrop-blur-[2px]"
        @mousedown.self="closePalette"
      >
        <div
          role="dialog"
          aria-modal="true"
          aria-label="全局搜索"
          class="w-full max-w-[620px] overflow-hidden rounded-xl border border-line bg-surface shadow-[0_24px_70px_-24px_rgba(16,24,40,0.45)]"
        >
          <div class="flex items-center gap-2.5 border-b border-line px-4">
            <AppIcon name="search" :size="16" class="shrink-0 text-faint" />
            <input
              ref="inputRef"
              v-model="query"
              type="text"
              placeholder="搜索房间、设备、镜像、课表或公告…"
              class="h-12 w-full bg-transparent text-[13px] text-ink placeholder:text-faint focus:outline-none"
            />
            <kbd
              class="shrink-0 rounded border border-line bg-canvas px-1.5 py-0.5 font-mono text-[10px] text-faint"
            >
              ESC
            </kbd>
          </div>

          <CommandPaletteResults
            :entries="entries"
            :active-index="activeIndex"
            :loading="loading"
            :query="query"
            @hover="activeIndex = $event"
            @select="(entry) => entry.action()"
          />

          <div
            class="flex items-center justify-between border-t border-line bg-canvas/40 px-4 py-2 text-[10px] text-faint"
          >
            <span>↑↓ 选择 · ↵ 打开 · Esc 关闭</span>
            <span>{{ entries.length }} 条结果</span>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.palette-enter-active,
.palette-leave-active {
  transition: opacity 0.15s ease;
}

.palette-enter-from,
.palette-leave-to {
  opacity: 0;
}
</style>
