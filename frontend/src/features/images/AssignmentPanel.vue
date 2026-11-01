<script setup lang="ts">
import { computed, ref } from 'vue'

import { api, isForbidden } from '@/api'
import type { ArtifactStatus, ImageAssignment, ImageChannel, Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import CopyText from '@/components/ui/CopyText.vue'
import FormField from '@/components/ui/FormField.vue'
import InfoList from '@/components/ui/InfoList.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, shortId } from '@/utils/format'
import { IMAGE_CHANNEL_OPTIONS, artifactStatusMeta, channelMeta } from './labels'

const props = defineProps<{
  imageId: string
  imageName: string
  status: ArtifactStatus
  rooms: Room[]
  roomsLoading: boolean
  roomsError: string
}>()

const toast = useToastStore()

const selectedRoomId = ref('')
const channel = ref<ImageChannel | ''>('')
const assigning = ref(false)
const assignError = ref('')
const result = ref<ImageAssignment | null>(null)

const unassignOpen = ref(false)
const unassigning = ref(false)
const unassignError = ref('')

const statusMeta = computed(() => artifactStatusMeta(props.status))
const assignable = computed(() => props.status === 'published')

const selectedRoom = computed(
  () => props.rooms.find((room) => room.id === selectedRoomId.value) ?? null,
)

function roomLabel(roomId: string | null | undefined): string {
  if (!roomId) return '—'
  const room = props.rooms.find((item) => item.id === roomId)
  return room ? `${room.name} · ${room.code}` : shortId(roomId)
}

const resultItems = computed(() => {
  if (!result.value) return []
  return [
    { label: 'assignment_id', value: result.value.id ?? '—', mono: true },
    { label: 'room_id', value: result.value.room_id ?? '—', mono: true },
    { label: '通道', value: channelMeta(result.value.channel).label },
    { label: 'rollout_state', value: result.value.rollout_state ?? '—' },
    { label: '分配时间', value: formatFullDateTime(result.value.assigned_at) },
    { label: 'assigned_by', value: shortId(result.value.assigned_by) },
  ]
})

async function assign() {
  if (assigning.value || !selectedRoomId.value) return

  assigning.value = true
  assignError.value = ''
  try {
    result.value = await api.assignImage(
      selectedRoomId.value,
      props.imageId,
      channel.value ? { channel: channel.value } : {},
    )
    toast.success('镜像已分配到房间', roomLabel(selectedRoomId.value))
  } catch (caught) {
    assignError.value = isForbidden(caught)
      ? '缺少 image:manage 权限，无法分配镜像。'
      : errorMessage(caught, '分配镜像失败')
  } finally {
    assigning.value = false
  }
}

async function unassign() {
  if (unassigning.value || !selectedRoomId.value) return

  unassigning.value = true
  unassignError.value = ''
  try {
    await api.unassignImage(selectedRoomId.value, props.imageId)
    unassignOpen.value = false
    result.value = null
    toast.success('已取消房间分配', roomLabel(selectedRoomId.value))
  } catch (caught) {
    unassignError.value = isForbidden(caught)
      ? '缺少 image:manage 权限，无法取消分配。'
      : errorMessage(caught, '取消分配失败')
    unassignOpen.value = false
    toast.error('取消分配失败', unassignError.value)
  } finally {
    unassigning.value = false
  }
}
</script>

<template>
  <PageSection title="房间分配" description="把已发布镜像分配到房间，后端会创建 download 分发任务">
    <div class="space-y-3 px-5 py-4">
      <p
        v-if="roomsError"
        class="rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-[11px] leading-5 text-amber-700"
      >
        {{ roomsError }}
      </p>

      <FormField label="房间" input-id="assign-room" required>
        <select id="assign-room" v-model="selectedRoomId" class="select" :disabled="roomsLoading">
          <option value="">{{ roomsLoading ? '房间加载中…' : '选择房间' }}</option>
          <option v-for="room in rooms" :key="room.id" :value="room.id">
            {{ room.name }} · {{ room.code }}
          </option>
        </select>
      </FormField>

      <FormField label="通道（可选）" input-id="assign-channel">
        <select id="assign-channel" v-model="channel" class="select" :disabled="!assignable">
          <option value="">不指定通道</option>
          <option v-for="option in IMAGE_CHANNEL_OPTIONS" :key="option.value" :value="option.value">
            {{ option.label }}
          </option>
        </select>
      </FormField>

      <p v-if="!assignable" class="text-[11px] leading-5 text-muted">
        只有已发布（published）的镜像可以分配到房间，当前状态为 {{ statusMeta.label }}。
      </p>

      <div class="flex items-center gap-2">
        <AppButton
          variant="primary"
          icon="plus"
          :disabled="!selectedRoomId || !assignable"
          :loading="assigning"
          @click="assign"
        >
          分配镜像
        </AppButton>
        <AppButton
          variant="danger"
          icon="x"
          :disabled="!selectedRoomId"
          @click="unassignOpen = true"
        >
          取消分配
        </AppButton>
      </div>

      <p
        v-if="assignError"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] leading-5 text-rose-700"
      >
        {{ assignError }}
      </p>
      <p
        v-if="unassignError"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] leading-5 text-rose-700"
      >
        {{ unassignError }}
      </p>

      <div v-if="result" class="rounded-md border border-line bg-canvas/50 px-3 py-2.5">
        <div class="flex items-center gap-2">
          <StateBadge label="分配成功" tone="emerald" />
          <span class="text-[11px] text-muted">{{ roomLabel(result.room_id) }}</span>
        </div>
        <InfoList class="mt-2" :items="resultItems" :columns="2">
          <template #assignment_id>
            <CopyText :value="result.id" :display="result.id" />
          </template>
          <template #room_id>
            <CopyText :value="result.room_id" :display="roomLabel(result.room_id)" :mono="false" />
          </template>
        </InfoList>
      </div>

      <p class="text-[10.5px] leading-5 text-faint">
        分配接口返回 ImageAssignment 记录；后端暂未提供分配列表接口，这里只展示最近一次操作结果。
      </p>
    </div>

    <ConfirmDialog
      :open="unassignOpen"
      title="取消房间分配"
      :description="`将取消「${imageName}」在所选房间上的分配。`"
      :detail="`房间：${selectedRoom ? `${selectedRoom.name} · ${selectedRoom.code}` : '未选择'}。取消后该房间将不再接收此镜像，已经完成的分发记录会保留。`"
      confirm-label="取消分配"
      danger
      :loading="unassigning"
      @confirm="unassign"
      @close="unassignOpen = false"
    />
  </PageSection>
</template>
