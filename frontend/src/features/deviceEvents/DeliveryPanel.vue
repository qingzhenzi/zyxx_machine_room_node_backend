<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type { PresenceDelivery } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import FilterTabs from '@/components/ui/FilterTabs.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, formatRelative } from '@/utils/format'

type DeliveryStatus = 'pending' | 'delivered' | 'failed'

const props = defineProps<{
  refreshToken: number
}>()

const status = ref('')
const deliveries = ref<PresenceDelivery[]>([])
const loading = ref(false)
const error = ref('')

const tabs = [
  { value: '', label: '全部' },
  { value: 'pending', label: '待投递' },
  { value: 'delivered', label: '已投递' },
  { value: 'failed', label: '失败' },
]

const statusMeta: Record<DeliveryStatus, { label: string; tone: 'amber' | 'emerald' | 'rose' }> = {
  pending: { label: '待投递', tone: 'amber' },
  delivered: { label: '已投递', tone: 'emerald' },
  failed: { label: '失败', tone: 'rose' },
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const data = await api.listPresenceDeliveries({
      status: status.value ? (status.value as DeliveryStatus) : undefined,
      limit: 50,
    })
    deliveries.value = data.items ?? []
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少权限：当前账号没有设备读取权限（device:read）。'
      : errorMessage(caught, '加载投递记录失败')
  } finally {
    loading.value = false
  }
}

function onStatusChange(value: string) {
  status.value = value
}

watch(status, () => void load())
watch(
  () => props.refreshToken,
  () => void load(),
)
onMounted(() => void load())
</script>

<template>
  <PageSection>
    <template #header>
      <div class="space-y-2.5">
        <div class="flex items-start justify-between gap-3">
          <div>
            <h2 class="text-[13.5px] font-semibold text-ink">投递记录</h2>
            <p class="mt-0.5 text-[11px] text-muted">在线事件 webhook 投递与重试记录（排障用）。</p>
          </div>
          <IconButton
            icon="refresh"
            label="刷新投递记录"
            variant="outline"
            :loading="loading"
            @click="load"
          />
        </div>
        <FilterTabs :model-value="status" :tabs="tabs" @change="onStatusChange" />
      </div>
    </template>

    <div
      v-if="error"
      role="alert"
      class="mx-5 mt-4 flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load">重试</AppButton>
    </div>

    <div v-if="loading && !deliveries.length" class="space-y-3 px-5 py-5">
      <div v-for="row in 4" :key="row" class="h-10 animate-pulse rounded-md bg-canvas" />
    </div>

    <ul v-else-if="deliveries.length" class="divide-y divide-line/70">
      <li v-for="delivery in deliveries" :key="delivery.id" class="px-5 py-3">
        <div class="flex items-center gap-2">
          <StateBadge
            :label="statusMeta[delivery.status].label"
            :tone="statusMeta[delivery.status].tone"
          />
          <span class="font-mono text-[10.5px] text-faint">事件 #{{ delivery.event_id }}</span>
          <span class="ml-auto text-[10px] text-faint">
            {{ formatRelative(delivery.last_attempt_at ?? delivery.next_attempt_at) }}
          </span>
        </div>
        <div
          class="mt-1.5 truncate font-mono text-[10.5px] text-ink-soft"
          :title="delivery.target_url"
        >
          {{ delivery.target_url }}
        </div>
        <div class="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[10px] text-faint">
          <span>尝试 {{ delivery.attempts }} 次</span>
          <span :title="formatFullDateTime(delivery.next_attempt_at)">
            下次重试 {{ formatRelative(delivery.next_attempt_at) }}
          </span>
        </div>
        <p
          v-if="delivery.last_error"
          class="mt-1.5 rounded border border-rose-100 bg-rose-50 px-2 py-1 text-[10.5px] leading-4 text-rose-700"
          :title="delivery.last_error"
        >
          {{ delivery.last_error }}
        </p>
      </li>
    </ul>

    <div v-else-if="!error" class="px-5 py-10 text-center text-[11.5px] text-muted">
      暂无投递记录
    </div>
  </PageSection>
</template>
