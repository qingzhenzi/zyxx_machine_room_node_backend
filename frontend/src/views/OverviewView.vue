<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'

import AppButton from '@/components/ui/AppButton.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import { useInterval } from '@/composables/useInterval'
import AuditPanel from '@/features/overview/AuditPanel.vue'
import PresencePanel from '@/features/overview/PresencePanel.vue'
import RecentEventsPanel from '@/features/overview/RecentEventsPanel.vue'
import StatCard from '@/features/overview/StatCard.vue'
import type { OverviewStat } from '@/features/overview/StatCard.vue'
import SystemPanel from '@/features/overview/SystemPanel.vue'
import TransfersPanel from '@/features/overview/TransfersPanel.vue'
import { useAuthStore } from '@/stores/auth'
import { useOverviewStore } from '@/stores/overview'
import { useToastStore } from '@/stores/toast'
import { downloadJson, formatPercent, formatRelative } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const overview = useOverviewStore()
const auth = useAuthStore()
const router = useRouter()
const toast = useToastStore()

const AUTO_KEY = 'rn:overview:auto-refresh'
const autoRefresh = ref(readAuto())
const today = new Date().toLocaleDateString('zh-CN', { month: 'long', day: 'numeric' })

const data = computed(() => overview.data)
const loading = computed(() => overview.loading)

const stats = computed<OverviewStat[]>(() => {
  const kpi = data.value?.kpi
  const onlineRate = kpi && kpi.devices > 0 ? (kpi.onlineDevices / kpi.devices) * 100 : null
  return [
    {
      label: '在线设备',
      value: kpi ? `${kpi.onlineDevices} / ${kpi.devices}` : '—',
      note: '最近 100 台设备中的在线数量',
      badge: onlineRate === null ? undefined : `在线率 ${formatPercent(onlineRate, 0)}`,
      tone: onlineRate === null ? 'slate' : onlineRate > 0 ? 'emerald' : 'rose',
      icon: 'monitor',
      loading: loading.value,
    },
    {
      label: '房间',
      value: kpi ? String(kpi.rooms) : '—',
      note: kpi ? `分布在 ${kpi.areas} 个区域` : '按区域组织的调度最小单位',
      badge: kpi ? `${kpi.schedules} 张课表` : undefined,
      tone: 'accent',
      icon: 'door',
      loading: loading.value,
    },
    {
      label: '今日课表事件',
      value: data.value ? String(data.value.todayEvents) : '—',
      note: '今日 00:00 起触发的排课事件',
      badge: data.value ? `${data.value.kpi.schedules} 张课表` : undefined,
      tone: 'sky',
      icon: 'calendar',
      loading: loading.value,
    },
    {
      label: '已发布镜像',
      value: kpi ? String(kpi.publishedImages) : '—',
      note: kpi ? `共 ${kpi.images} 个镜像制品` : '镜像制品与发布通道',
      badge: kpi ? `${kpi.images} 个制品` : undefined,
      tone: 'violet',
      icon: 'package',
      loading: loading.value,
    },
  ]
})

function readAuto(): boolean {
  try {
    return localStorage.getItem(AUTO_KEY) !== '0'
  } catch {
    return true
  }
}

function setAuto(value: boolean) {
  autoRefresh.value = value
  try {
    localStorage.setItem(AUTO_KEY, value ? '1' : '0')
  } catch {
    // 忽略
  }
}

function openDevice(serial: string) {
  void router.push({ name: 'device-detail', params: { serial } })
}

function exportOverview() {
  if (!data.value) return
  downloadJson('room-node-overview.json', data.value)
  toast.success('总览数据已导出')
}

useInterval(() => {
  if (autoRefresh.value) void overview.load()
}, 30_000)

onMounted(() => void overview.load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="总览" badge="实时">
      <template #description>
        {{ today }} · 最近同步于 {{ data ? formatRelative(data.updatedAt) : '—' }} · 数据来自管理
        API
      </template>
      <template #actions>
        <label class="flex items-center gap-2 text-[11.5px] text-muted">
          <input
            :checked="autoRefresh"
            type="checkbox"
            class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
            @change="setAuto(($event.target as HTMLInputElement).checked)"
          />
          自动刷新
        </label>
        <IconButton
          icon="refresh"
          label="刷新总览"
          variant="outline"
          :loading="overview.loading"
          @click="overview.load()"
        />
        <AppButton icon="download" :disabled="!data" @click="exportOverview">导出</AppButton>
      </template>
    </PageHeader>

    <div
      v-if="overview.error"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ overview.error }}</span>
      <AppButton size="sm" variant="ghost" @click="overview.load()">重试</AppButton>
    </div>

    <section class="mt-4 grid grid-cols-4 gap-4" aria-label="关键指标">
      <StatCard v-for="stat in stats" :key="stat.label" :stat="stat" />
    </section>

    <div class="mt-4 grid grid-cols-12 items-start gap-4">
      <div class="col-span-8 flex flex-col gap-4">
        <PresencePanel :events="data?.recentPresence ?? []" :loading="loading" @open="openDevice" />
        <TransfersPanel
          v-if="auth.can(PERMISSIONS.storageRead)"
          :transfers="data?.transfers ?? []"
        />
        <RecentEventsPanel :events="data?.recentEvents ?? []" :loading="loading" />
      </div>

      <div class="col-span-4 flex flex-col gap-4">
        <SystemPanel />
        <AuditPanel v-if="auth.can(PERMISSIONS.auditRead)" :logs="data?.recentAudit ?? []" />
        <section
          v-if="auth.can(PERMISSIONS.announcementRead) && data?.announcements.length"
          class="rounded-xl border border-line bg-surface"
        >
          <header class="border-b border-line px-5 py-4">
            <h2 class="text-[13.5px] font-semibold text-ink">最新公告</h2>
            <p class="mt-0.5 text-[11px] text-muted">后台公告（含草稿与归档）</p>
          </header>
          <ul class="divide-y divide-line/70">
            <li
              v-for="announcement in data.announcements"
              :key="announcement.id"
              class="flex items-center gap-2 px-5 py-2.5"
            >
              <span
                class="rounded px-1.5 py-0.5 text-[10px] font-medium"
                :class="
                  announcement.status === 'published'
                    ? 'bg-emerald-50 text-emerald-700'
                    : announcement.status === 'draft'
                      ? 'bg-amber-50 text-amber-700'
                      : 'bg-slate-100 text-slate-600'
                "
              >
                {{ announcement.status }}
              </span>
              <span class="min-w-0 flex-1 truncate text-[11.5px] text-ink">
                {{ announcement.title }}
              </span>
              <span class="shrink-0 text-[10px] text-faint">
                {{ formatRelative(announcement.updated_at) }}
              </span>
            </li>
          </ul>
        </section>
      </div>
    </div>
  </div>
</template>
