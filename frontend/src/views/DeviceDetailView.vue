<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api, isForbidden, isNotFound } from '@/api'
import type { DeviceInfo } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import InfoList from '@/components/ui/InfoList.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { deviceOnlineMeta, deviceStatusMeta } from '@/features/devices/deviceMeta'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, formatRelative, shortId } from '@/utils/format'

const route = useRoute()
const router = useRouter()

const device = ref<DeviceInfo | null>(null)
const loading = ref(false)
const error = ref('')
const notFound = ref(false)

const serial = computed(() => String(route.params.serial ?? ''))
const statusMeta = computed(() => deviceStatusMeta(device.value?.status))
const onlineMeta = computed(() => deviceOnlineMeta(device.value?.online))

const infoItems = computed(() => {
  const data = device.value
  if (!data) return []
  return [
    { label: '序列号', value: data.serial, mono: true },
    { label: '设备槽位', value: data.device_slot, mono: true },
    { label: '注册状态', value: statusMeta.value.label },
    { label: '在线状态', value: onlineMeta.value.label },
    { label: '镜像下载', value: data.downloaded ? '已下载' : '未下载' },
    { label: '签发时间', value: formatFullDateTime(data.issued_at) },
    { label: '激活时间', value: formatFullDateTime(data.activated_at) },
    { label: '吊销时间', value: formatFullDateTime(data.revoked_at) },
    { label: '最后心跳', value: formatRelative(data.last_seen_at) },
  ]
})

function openEvents() {
  void router.push({ name: 'device-events' })
}

function backToList() {
  void router.push({ name: 'devices' })
}

async function load() {
  if (!serial.value) return
  loading.value = true
  error.value = ''
  notFound.value = false
  try {
    device.value = await api.getDevice(serial.value)
  } catch (caught) {
    device.value = null
    if (isNotFound(caught)) {
      notFound.value = true
    } else if (isForbidden(caught)) {
      error.value = '缺少权限：当前账号没有设备读取权限（device:read）。'
    } else {
      error.value = errorMessage(caught, '加载设备详情失败')
    }
  } finally {
    loading.value = false
  }
}

watch(serial, () => void load())
onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="设备详情" :badge="device ? statusMeta.label : ''">
      <template #description>
        <RouterLink :to="{ name: 'devices' }" class="link">← 返回设备列表</RouterLink>
        <span v-if="device" class="ml-2 inline-flex items-center gap-1">
          · 序列号
          <CopyText :value="device.serial" :display="shortId(device.serial, 16)" label="序列号" />
        </span>
      </template>
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新设备详情"
          variant="outline"
          :loading="loading"
          @click="load"
        />
        <AppButton icon="wifi" @click="openEvents">查看在线事件</AppButton>
      </template>
    </PageHeader>

    <div
      v-if="error && device"
      role="alert"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load">重试</AppButton>
    </div>

    <div v-if="loading && !device" class="mt-4 grid grid-cols-12 gap-4">
      <div class="col-span-7 h-80 animate-pulse rounded-xl border border-line bg-surface" />
      <div class="col-span-5 h-80 animate-pulse rounded-xl border border-line bg-surface" />
    </div>

    <EmptyState
      v-else-if="notFound"
      class="mt-4 rounded-xl border border-line bg-surface"
      title="设备不存在"
      description="该序列号对应的设备可能尚未注册，或链接中的序列号不正确。"
      icon="monitor"
    >
      <template #action>
        <AppButton icon="arrow-left" @click="backToList">返回设备列表</AppButton>
      </template>
    </EmptyState>

    <EmptyState
      v-else-if="error && !device"
      class="mt-4 rounded-xl border border-line bg-surface"
      title="加载设备详情失败"
      :description="error"
      icon="alert"
    >
      <template #action>
        <AppButton icon="refresh" @click="load">重试</AppButton>
      </template>
    </EmptyState>

    <template v-else-if="device">
      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <div class="col-span-7 flex flex-col gap-4">
          <PageSection title="设备信息" description="注册、激活、吊销与在线状态。">
            <div class="px-5 py-4">
              <div class="mb-4 flex flex-wrap items-center gap-4 border-b border-line/70 pb-3.5">
                <div class="flex items-center gap-1.5">
                  <span class="text-[10.5px] text-faint">注册状态</span>
                  <StateBadge :label="statusMeta.label" :tone="statusMeta.tone" />
                </div>
                <div class="flex items-center gap-1.5">
                  <span class="text-[10.5px] text-faint">在线状态</span>
                  <StateBadge
                    :label="onlineMeta.label"
                    :tone="onlineMeta.tone"
                    :pulse="device.online === 'online'"
                  />
                </div>
                <div class="flex items-center gap-1.5">
                  <span class="text-[10.5px] text-faint">镜像</span>
                  <StateBadge
                    :label="device.downloaded ? '已下载' : '未下载'"
                    :tone="device.downloaded ? 'sky' : 'slate'"
                  />
                </div>
                <span class="ml-auto text-[11px] text-faint">
                  最后心跳：{{ formatRelative(device.last_seen_at) }}
                </span>
              </div>

              <InfoList :items="infoItems" :columns="2" />
            </div>
          </PageSection>

          <PageSection title="客户端信息" description="设备注册时上报的 client_info（JSON）。">
            <div class="px-5 py-4">
              <p v-if="!device.client_info" class="mb-2 text-[11.5px] text-muted">
                设备尚未上报客户端信息。
              </p>
              <JsonBlock :value="device.client_info" label="client_info" collapsible />
            </div>
          </PageSection>
        </div>

        <div class="col-span-5 flex flex-col gap-4">
          <PageSection title="归属" description="设备当前关联的房间与区域。">
            <div class="space-y-3 px-5 py-4">
              <div>
                <div class="text-[10.5px] text-faint">所属房间</div>
                <div class="mt-0.5 text-[12px]">
                  <RouterLink
                    v-if="device.room_id"
                    :to="{ name: 'room-detail', params: { id: device.room_id } }"
                    class="link"
                  >
                    {{ device.room_name ?? '房间' }}
                    <span v-if="device.room_code" class="font-mono text-[10.5px] text-faint">
                      ({{ device.room_code }})
                    </span>
                  </RouterLink>
                  <span v-else class="text-faint">未绑定房间</span>
                </div>
              </div>
              <div>
                <div class="text-[10.5px] text-faint">所属区域</div>
                <div class="mt-0.5 text-[12px]">
                  <RouterLink
                    v-if="device.area_id"
                    :to="{ name: 'rooms', query: { area_id: device.area_id } }"
                    class="link"
                  >
                    {{ device.area_name ?? '区域' }}
                  </RouterLink>
                  <span v-else class="text-faint">—</span>
                </div>
              </div>
              <div>
                <div class="text-[10.5px] text-faint">设备槽位</div>
                <div class="mt-0.5 font-mono text-[11.5px] text-ink">
                  {{ device.device_slot }}
                </div>
              </div>
            </div>
          </PageSection>

          <PageSection title="快捷操作" description="前往在线事件或房间详情排查问题。">
            <div class="flex flex-col gap-2 px-5 py-4">
              <RouterLink
                :to="{ name: 'device-events' }"
                class="flex h-9 items-center justify-between rounded-md border border-line bg-surface px-3 text-[12px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
              >
                查看设备在线事件
                <span class="text-[10.5px] text-faint">/device-events</span>
              </RouterLink>
              <RouterLink
                v-if="device.room_id"
                :to="{ name: 'room-detail', params: { id: device.room_id } }"
                class="flex h-9 items-center justify-between rounded-md border border-line bg-surface px-3 text-[12px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
              >
                打开所属房间
                <span class="text-[10.5px] text-faint">/rooms/{{ shortId(device.room_id) }}</span>
              </RouterLink>
            </div>
          </PageSection>
        </div>
      </div>
    </template>
  </div>
</template>
