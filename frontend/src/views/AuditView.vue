<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api, isForbidden } from '@/api'
import type { AuditLog } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import AuditTable from '@/features/audit/AuditTable.vue'
import { ACTION_GROUPS, RESOURCE_TYPE_OPTIONS, actionLabel } from '@/features/audit/auditOptions'
import { errorMessage } from '@/utils/errors'
import { formatRelative } from '@/utils/format'

const PAGE_SIZE = 20

const logs = ref<AuditLog[]>([])
const total = ref(0)
const page = ref(1)
const action = ref('')
const resourceType = ref('')
const loading = ref(false)
const error = ref('')
const refreshedAt = ref<string | null>(null)

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const hasFilters = computed(() => action.value !== '' || resourceType.value !== '')
const activeFilterLabel = computed(() => {
  const parts: string[] = []
  if (action.value) parts.push(actionLabel(action.value))
  if (resourceType.value) {
    const option = RESOURCE_TYPE_OPTIONS.find((item) => item.value === resourceType.value)
    parts.push(option?.label ?? resourceType.value)
  }
  return parts.join(' · ')
})

async function load() {
  loading.value = true
  error.value = ''
  try {
    const result = await api.listAuditLogs({
      page: page.value,
      page_size: PAGE_SIZE,
      action: action.value || undefined,
      resource_type: resourceType.value || undefined,
    })
    logs.value = result.items ?? []
    total.value = result.total ?? 0
    refreshedAt.value = new Date().toISOString()
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少 audit:read 权限，无法读取审计日志。'
      : errorMessage(caught, '加载审计日志失败')
    logs.value = []
    total.value = 0
  } finally {
    loading.value = false
  }
}

function applyFilters() {
  page.value = 1
  void load()
}

function resetFilters() {
  action.value = ''
  resourceType.value = ''
  applyFilters()
}

function changePage(next: number) {
  page.value = next
  void load()
}

onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="审计日志" description="管理操作与资源变更记录，展开行可查看 detail JSON。">
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新审计日志"
          variant="outline"
          :loading="loading"
          @click="load()"
        />
      </template>
    </PageHeader>

    <div class="mt-4 flex flex-wrap items-center justify-between gap-3">
      <div class="flex items-center gap-2">
        <select
          v-model="action"
          class="select w-[218px]"
          aria-label="按操作筛选"
          @change="applyFilters"
        >
          <option value="">全部操作</option>
          <optgroup v-for="group in ACTION_GROUPS" :key="group.label" :label="group.label">
            <option v-for="option in group.actions" :key="option.value" :value="option.value">
              {{ option.label }}（{{ option.value }}）
            </option>
          </optgroup>
        </select>

        <select
          v-model="resourceType"
          class="select w-[168px]"
          aria-label="按资源类型筛选"
          @change="applyFilters"
        >
          <option value="">全部资源</option>
          <option v-for="option in RESOURCE_TYPE_OPTIONS" :key="option.value" :value="option.value">
            {{ option.label }}（{{ option.value }}）
          </option>
        </select>

        <AppButton v-if="hasFilters" size="sm" @click="resetFilters">重置筛选</AppButton>
      </div>

      <p class="text-[11px] text-muted">
        共 {{ total }} 条记录
        <span v-if="hasFilters"> · 当前筛选：{{ activeFilterLabel }}</span>
        <span v-if="refreshedAt"> · 同步于 {{ formatRelative(refreshedAt) }}</span>
      </p>
    </div>

    <div
      v-if="error"
      class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <div class="mt-3">
      <AuditTable :logs="logs" :loading="loading">
        <template #footer>
          <PaginationBar
            v-if="total > 0"
            :page="page"
            :total-pages="totalPages"
            :total="total"
            label="条审计记录"
            @change="changePage"
          />
        </template>
      </AuditTable>
    </div>
  </div>
</template>
