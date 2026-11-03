<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import { api, isForbidden } from '@/api'
import type { TicketStats } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import FilterTabs from '@/components/ui/FilterTabs.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import TicketAssigneeLoad from '@/features/tickets/TicketAssigneeLoad.vue'
import TicketCategoryBars from '@/features/tickets/TicketCategoryBars.vue'
import TicketRankList from '@/features/tickets/TicketRankList.vue'
import TicketStatsCards from '@/features/tickets/TicketStatsCards.vue'
import TicketTrendChart from '@/features/tickets/TicketTrendChart.vue'
import {
  buildDailySeries,
  buildInsight,
  buildStatsRange,
  normalizeTotals,
  toAssigneeRows,
  toCategoryRows,
} from '@/features/tickets/ticketStats'
import { useAuthStore } from '@/stores/auth'
import { errorMessage } from '@/utils/errors'
import { shortId } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const auth = useAuthStore()
const router = useRouter()

const RANGE_TABS = [
  { value: '7', label: '近 7 天' },
  { value: '30', label: '近 30 天' },
  { value: '90', label: '近 90 天' },
]

const days = ref(30)
const stats = ref<TicketStats | null>(null)
const loading = ref(false)
const error = ref('')
const forbidden = ref(false)

const canRead = computed(() => auth.can(PERMISSIONS.ticketRead))
const totals = computed(() => normalizeTotals(stats.value?.totals))
const series = computed(() =>
  buildDailySeries(stats.value?.by_day, stats.value?.from, stats.value?.to),
)
const categoryRows = computed(() => toCategoryRows(stats.value?.by_category))
const assigneeRows = computed(() => toAssigneeRows(stats.value?.by_assignee))

const topRooms = computed(() =>
  (stats.value?.top_rooms ?? []).slice(0, 5).map((item) => ({
    name: item.room_name || shortId(item.room_id),
    sub: item.room_id ? shortId(item.room_id) : null,
    count: item.count ?? 0,
  })),
)

const topDevices = computed(() =>
  (stats.value?.top_devices ?? []).slice(0, 5).map((item) => ({
    name: item.device_slot || item.serial || '未知设备',
    sub: item.serial || null,
    count: item.count ?? 0,
  })),
)

const insight = computed(() =>
  buildInsight({
    days: days.value,
    totals: totals.value,
    byCategory: categoryRows.value,
    topRooms: stats.value?.top_rooms ?? [],
  }),
)

const showSkeleton = computed(() => loading.value && stats.value === null)

async function load() {
  loading.value = true
  error.value = ''
  forbidden.value = false
  try {
    const { from, to } = buildStatsRange(days.value)
    stats.value = await api.ticketStats({ from, to })
  } catch (caught) {
    stats.value = null
    if (isForbidden(caught)) {
      forbidden.value = true
    } else {
      error.value = errorMessage(caught, '加载工单分析数据失败')
    }
  } finally {
    loading.value = false
  }
}

function changeRange(value: string) {
  days.value = Number(value)
}

function refresh() {
  void load()
}

watch(days, () => void load())

onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="工单分析"
      description="按时间窗口统计工单新增、关闭与 SLA 表现，定位高频故障房间与处理负载。"
    >
      <template #actions>
        <FilterTabs :model-value="String(days)" :tabs="RANGE_TABS" @change="changeRange" />
        <IconButton
          icon="refresh"
          label="刷新工单分析"
          variant="outline"
          :loading="loading"
          @click="refresh"
        />
        <AppButton icon="ticket" @click="router.push({ name: 'tickets' })">工单列表</AppButton>
      </template>
    </PageHeader>

    <div v-if="forbidden || !canRead" class="mt-4 rounded-xl border border-line bg-surface">
      <EmptyState
        title="缺少工单权限"
        description="当前账号没有 ticket:read 权限，无法查看工单分析。请联系管理员授予工单读取权限。"
        icon="lock"
      />
    </div>

    <div v-else-if="error && !stats" class="mt-4 rounded-xl border border-line bg-surface">
      <EmptyState title="加载工单分析失败" :description="error" icon="alert">
        <template #action>
          <AppButton icon="refresh" @click="refresh">重试</AppButton>
        </template>
      </EmptyState>
    </div>

    <template v-else>
      <div
        v-if="error"
        role="alert"
        class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
      >
        <span>{{ error }}</span>
        <AppButton size="sm" variant="ghost" @click="refresh">重试</AppButton>
      </div>

      <TicketStatsCards class="mt-4" :totals="totals" :days="days" :loading="showSkeleton" />

      <div class="mt-4 grid grid-cols-12 items-stretch gap-4">
        <PageSection
          class="col-span-8"
          title="新增 vs 关闭（按日）"
          description="柱状图由纯 CSS 渲染，悬停可查看当日明细"
        >
          <TicketTrendChart :series="series" :loading="showSkeleton" />
        </PageSection>
        <PageSection class="col-span-4" title="分类分布" :description="`近 ${days} 天新增工单`">
          <TicketCategoryBars :items="categoryRows" :loading="showSkeleton" />
        </PageSection>
      </div>

      <div class="mt-4 grid grid-cols-12 items-stretch gap-4">
        <PageSection class="col-span-4" title="房间 TOP5" description="按工单量排序">
          <TicketRankList
            :items="topRooms"
            :loading="showSkeleton"
            empty-text="所选区间暂无房间数据"
          />
        </PageSection>
        <PageSection class="col-span-4" title="设备 TOP5" description="按工单量排序">
          <TicketRankList
            :items="topDevices"
            :loading="showSkeleton"
            empty-text="所选区间暂无设备数据"
          />
        </PageSection>
        <PageSection class="col-span-4" title="处理人负载" description="未关闭 vs 已解决">
          <TicketAssigneeLoad :items="assigneeRows" :loading="showSkeleton" />
        </PageSection>
      </div>

      <section
        class="mt-4 flex items-start gap-3 rounded-xl border border-line bg-surface px-5 py-4"
      >
        <span
          class="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-full border border-line bg-canvas text-faint"
        >
          <AppIcon name="spark" :size="14" />
        </span>
        <div class="min-w-0">
          <p class="text-[12px] font-semibold text-ink">{{ insight.title }}</p>
          <p class="mt-1 text-[11.5px] leading-5 text-muted">{{ insight.text }}</p>
          <p class="mt-1.5 text-[10px] text-faint">
            洞察由当前时间窗口的统计结果自动生成（新增 / 分类 / 房间），不硬编码结论。
          </p>
        </div>
      </section>
    </template>
  </div>
</template>
