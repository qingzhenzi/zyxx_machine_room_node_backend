<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api, isForbidden } from '@/api'
import type { Collection, Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { collectionKindLabel, collectionStatusMeta } from '@/features/storage/labels'
import { errorMessage } from '@/utils/errors'
import { formatBytes, formatDateTime, shortId } from '@/utils/format'

const props = defineProps<{
  rooms: Room[]
  roomsLoading: boolean
  roomsError: string
}>()

const PAGE_SIZE = 20

const items = ref<Collection[]>([])
const page = ref(1)
const total = ref(0)
const loading = ref(false)
const error = ref('')
const roomId = ref('')

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))

function roomLabel(id: string | null | undefined): string {
  if (!id) return '—'
  const room = props.rooms.find((item) => item.id === id)
  return room ? `${room.name} · ${room.code}` : shortId(id)
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const data = await api.listCollections({
      room_id: roomId.value || undefined,
      page: page.value,
      page_size: PAGE_SIZE,
    })
    items.value = data.items ?? []
    total.value = data.total ?? 0
    if (data.page) page.value = data.page
  } catch (caught) {
    items.value = []
    total.value = 0
    error.value = isForbidden(caught)
      ? '缺少 storage:read 权限，无法加载收集登记。'
      : errorMessage(caught, '加载收集登记失败')
  } finally {
    loading.value = false
  }
}

function onRoomChange() {
  page.value = 1
  void load()
}

function changePage(next: number) {
  page.value = next
  void load()
}

onMounted(() => void load())
</script>

<template>
  <PageSection>
    <template #header>
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 class="text-[13.5px] font-semibold text-ink">收集登记</h2>
          <p class="mt-0.5 text-[11px] text-muted">从房间节点收集上来的产物登记，按创建时间倒序</p>
        </div>
        <div class="flex items-center gap-2">
          <select
            v-model="roomId"
            class="select w-[200px]"
            aria-label="按房间筛选收集登记"
            :disabled="roomsLoading"
            @change="onRoomChange"
          >
            <option value="">全部房间</option>
            <option v-for="room in rooms" :key="room.id" :value="room.id">
              {{ room.name }} · {{ room.code }}
            </option>
          </select>
          <IconButton
            icon="refresh"
            label="刷新收集登记"
            variant="outline"
            :loading="loading"
            @click="load"
          />
        </div>
      </div>
    </template>

    <p
      v-if="roomsError"
      class="border-b border-line bg-amber-50 px-5 py-2 text-[11px] text-amber-700"
    >
      {{ roomsError }}
    </p>

    <div v-if="error" class="flex items-center justify-between px-5 py-3">
      <p class="text-[11.5px] text-rose-700">{{ error }}</p>
      <AppButton size="sm" variant="ghost" @click="load">重试</AppButton>
    </div>

    <div class="overflow-x-auto">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">状态</th>
            <th class="table-th">sha256</th>
            <th class="table-th">大小</th>
            <th class="table-th">kind</th>
            <th class="table-th">房间</th>
            <th class="table-th pr-5">创建时间</th>
          </tr>
        </thead>

        <tbody v-if="loading">
          <tr v-for="row in 5" :key="row" class="border-b border-line/70 last:border-b-0">
            <td v-for="cell in 6" :key="cell" class="table-td" :class="cell === 1 ? 'pl-5' : ''">
              <div class="h-3 w-20 animate-pulse rounded bg-line" />
            </td>
          </tr>
        </tbody>

        <tbody v-else-if="items.length">
          <tr
            v-for="collection in items"
            :key="collection.id"
            class="border-b border-line/70 last:border-b-0 hover:bg-canvas/50"
          >
            <td class="table-td pl-5">
              <StateBadge
                :label="collectionStatusMeta(collection.status).label"
                :tone="collectionStatusMeta(collection.status).tone"
              />
            </td>
            <td class="table-td">
              <CopyText
                :value="collection.sha256"
                :display="shortId(collection.sha256, 16)"
                label="sha256"
              />
            </td>
            <td class="table-td text-[11px]">{{ formatBytes(collection.size) }}</td>
            <td class="table-td text-[11px]">{{ collectionKindLabel(collection.kind) }}</td>
            <td class="table-td text-[11px]">{{ roomLabel(collection.room_id) }}</td>
            <td class="table-td pr-5 text-[11px]">
              {{ formatDateTime(collection.created_at) }}
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <EmptyState
      v-if="!loading && !items.length && !error"
      title="暂无收集登记"
      description="房间节点上报收集产物后，会在这里登记 sha256 / size / kind 与校验状态。"
      icon="database"
    />

    <PaginationBar
      v-if="!loading && total > 0"
      :page="page"
      :total-pages="totalPages"
      :total="total"
      label="条收集记录"
      @change="changePage"
    />
  </PageSection>
</template>
