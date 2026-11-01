<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink, useRoute } from 'vue-router'

import AppIcon from '@/components/ui/AppIcon.vue'
import { useAuthStore } from '@/stores/auth'
import { useShellStore } from '@/stores/shell'
import { initials } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const route = useRoute()
const auth = useAuthStore()
const shell = useShellStore()

interface NavItem {
  name: string
  label: string
  icon: string
  to: string
  permission?: string
}

const navGroups: { label: string; items: NavItem[] }[] = [
  {
    label: '工作台',
    items: [{ name: 'overview', label: '总览', icon: 'overview', to: '/' }],
  },
  {
    label: '空间与设备',
    items: [
      {
        name: 'areas',
        label: '区域',
        icon: 'layers',
        to: '/areas',
        permission: PERMISSIONS.areaRead,
      },
      {
        name: 'rooms',
        label: '房间',
        icon: 'door',
        to: '/rooms',
        permission: PERMISSIONS.roomRead,
      },
      {
        name: 'devices',
        label: '设备',
        icon: 'monitor',
        to: '/devices',
        permission: PERMISSIONS.deviceRead,
      },
      {
        name: 'device-events',
        label: '在线事件',
        icon: 'wifi',
        to: '/device-events',
        permission: PERMISSIONS.deviceRead,
      },
    ],
  },
  {
    label: '机器',
    items: [
      {
        name: 'machines',
        label: '机器总览',
        icon: 'cpu',
        to: '/machines',
        permission: PERMISSIONS.deviceRead,
      },
    ],
  },
  {
    label: '课表与事件',
    items: [
      {
        name: 'schedules',
        label: '课表',
        icon: 'calendar',
        to: '/schedules',
        permission: PERMISSIONS.scheduleRead,
      },
      {
        name: 'events',
        label: '触发事件',
        icon: 'zap',
        to: '/events',
        permission: PERMISSIONS.scheduleEventRead,
      },
    ],
  },
  {
    label: '资源',
    items: [
      {
        name: 'images',
        label: '镜像',
        icon: 'package',
        to: '/images',
        permission: PERMISSIONS.imageRead,
      },
      {
        name: 'storage',
        label: '存储',
        icon: 'database',
        to: '/storage',
        permission: PERMISSIONS.storageRead,
      },
      {
        name: 'agent-configs',
        label: '节点配置',
        icon: 'sliders',
        to: '/agent-configs',
        permission: PERMISSIONS.deviceRead,
      },
    ],
  },
  {
    label: '管理',
    items: [
      {
        name: 'users',
        label: '用户',
        icon: 'users',
        to: '/users',
        permission: PERMISSIONS.userManage,
      },
      {
        name: 'roles',
        label: '角色',
        icon: 'shield',
        to: '/roles',
        permission: PERMISSIONS.roleManage,
      },
      {
        name: 'audit',
        label: '审计日志',
        icon: 'scroll',
        to: '/audit',
        permission: PERMISSIONS.auditRead,
      },
      {
        name: 'announcements',
        label: '公告',
        icon: 'megaphone',
        to: '/announcements',
        permission: PERMISSIONS.announcementRead,
      },
    ],
  },
]

const visibleGroups = computed(() =>
  navGroups
    .map((group) => ({
      ...group,
      items: group.items.filter((item) => !item.permission || auth.can(item.permission)),
    }))
    .filter((group) => group.items.length > 0),
)

const isActive = (name: string) => route.name === name || route.meta.activeName === name
const userInitials = computed(() => initials(auth.displayName))
const roleLabel = computed(() => auth.roles.join(' / ') || '成员')
</script>

<template>
  <aside class="flex min-h-0 flex-col border-r border-line bg-surface">
    <div class="flex h-14 shrink-0 items-center border-b border-line px-5">
      <RouterLink to="/" class="flex min-w-0 items-baseline gap-2">
        <span class="truncate text-[15px] font-semibold tracking-[-0.015em] text-ink">
          Room Node
        </span>
        <span
          class="shrink-0 rounded border border-line px-1.5 py-px font-mono text-[9.5px] leading-4 text-faint"
        >
          console
        </span>
      </RouterLink>
    </div>

    <div class="shrink-0 px-3 pt-3">
      <div class="rounded-lg border border-line bg-canvas/60 px-3 py-2">
        <p class="truncate text-[12.5px] font-medium text-ink">房间节点管理</p>
        <p class="mt-0.5 truncate text-[10.5px] text-muted">
          {{ shell.rooms }} 个房间 · {{ shell.devices }} 台设备
          <span v-if="shell.onlineDevices">· {{ shell.onlineDevices }} 在线</span>
        </p>
      </div>
    </div>

    <nav aria-label="主导航" class="min-h-0 flex-1 overflow-y-auto px-3 py-4">
      <div v-for="group in visibleGroups" :key="group.label" class="mb-5 last:mb-0">
        <p class="mb-1.5 px-2.5 text-[10px] font-medium tracking-[0.08em] text-faint">
          {{ group.label }}
        </p>
        <ul class="space-y-0.5">
          <li v-for="item in group.items" :key="item.name">
            <RouterLink
              :to="item.to"
              class="group flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] transition-colors"
              :class="
                isActive(item.name)
                  ? 'bg-accent-soft font-medium text-accent'
                  : 'text-ink-soft hover:bg-canvas hover:text-ink'
              "
            >
              <AppIcon
                :name="item.icon"
                :size="16"
                :class="isActive(item.name) ? 'text-accent' : 'text-faint group-hover:text-muted'"
              />
              <span class="truncate">{{ item.label }}</span>
            </RouterLink>
          </li>
        </ul>
      </div>
    </nav>

    <div class="shrink-0 border-t border-line p-3">
      <div class="flex items-center gap-2.5 rounded-md px-2 py-1.5">
        <span
          class="flex h-7 w-7 items-center justify-center rounded-full bg-accent-soft text-[10px] font-semibold text-accent ring-1 ring-accent-line/70"
        >
          {{ userInitials }}
        </span>
        <span class="min-w-0 flex-1">
          <span class="block truncate text-[12px] font-medium text-ink">
            {{ auth.displayName }}
          </span>
          <span class="block truncate text-[10.5px] text-muted">{{ roleLabel }}</span>
        </span>
        <RouterLink
          to="/account"
          aria-label="账户设置"
          class="rounded p-1 text-faint transition-colors hover:bg-canvas hover:text-muted"
        >
          <AppIcon name="settings" :size="15" />
        </RouterLink>
      </div>
    </div>
  </aside>
</template>
