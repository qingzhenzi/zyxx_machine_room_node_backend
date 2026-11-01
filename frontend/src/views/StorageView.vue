<script setup lang="ts">
import { onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api, isForbidden } from '@/api'
import type { Room } from '@/api/schema'
import FilterTabs from '@/components/ui/FilterTabs.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import CollectionsPanel from '@/features/storage/CollectionsPanel.vue'
import StorageObjectPanel from '@/features/storage/StorageObjectPanel.vue'
import TransfersPanel from '@/features/storage/TransfersPanel.vue'
import { errorMessage } from '@/utils/errors'

type StorageTab = 'objects' | 'transfers' | 'collections'

const TAB_VALUES: StorageTab[] = ['objects', 'transfers', 'collections']

const tabs: { value: StorageTab; label: string }[] = [
  { value: 'objects', label: '对象' },
  { value: 'transfers', label: '分发任务' },
  { value: 'collections', label: '收集登记' },
]

const route = useRoute()
const router = useRouter()

function readTab(): StorageTab {
  const raw = route.query.tab
  const value = Array.isArray(raw) ? raw[0] : raw
  return value && TAB_VALUES.includes(value as StorageTab) ? (value as StorageTab) : 'objects'
}

const tab = ref<StorageTab>(readTab())

const rooms = ref<Room[]>([])
const roomsLoading = ref(false)
const roomsError = ref('')

async function loadRooms() {
  roomsLoading.value = true
  roomsError.value = ''
  try {
    const data = await api.listRooms({ page_size: 100 })
    rooms.value = data.items ?? []
  } catch (caught) {
    roomsError.value = isForbidden(caught)
      ? '缺少 room:read 权限，房间筛选不可用。'
      : errorMessage(caught, '加载房间列表失败，房间筛选不可用。')
  } finally {
    roomsLoading.value = false
  }
}

function onTabChange(value: string) {
  const next = TAB_VALUES.includes(value as StorageTab) ? (value as StorageTab) : 'objects'
  tab.value = next
  void router.replace({ query: { ...route.query, tab: next } })
}

watch(
  () => route.query.tab,
  () => {
    const next = readTab()
    if (next !== tab.value) tab.value = next
  },
)

onMounted(() => void loadRooms())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="存储" description="S3 对象预签名上传/登记、镜像分发任务与房间收集登记。">
      <template #actions>
        <FilterTabs :model-value="tab" :tabs="tabs" @change="onTabChange" />
      </template>
    </PageHeader>

    <div class="mt-4">
      <StorageObjectPanel v-if="tab === 'objects'" />
      <TransfersPanel
        v-else-if="tab === 'transfers'"
        :rooms="rooms"
        :rooms-loading="roomsLoading"
        :rooms-error="roomsError"
      />
      <CollectionsPanel
        v-else
        :rooms="rooms"
        :rooms-loading="roomsLoading"
        :rooms-error="roomsError"
      />
    </div>
  </div>
</template>
