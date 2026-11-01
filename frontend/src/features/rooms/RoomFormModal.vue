<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type { Area, CreateRoomRequest, Room, UpdateRoomRequest } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'

interface FormErrors {
  area_id?: string
  code?: string
  name?: string
  capacity?: string
  metadata?: string
}

const props = defineProps<{
  open: boolean
  room: Room | null
  areas: Area[]
}>()

const emit = defineEmits<{
  close: []
  saved: [room: Room]
}>()

const toast = useToastStore()

const areaId = ref('')
const code = ref('')
const name = ref('')
const capacity = ref('0')
const deviceSlot = ref('')
const webhookUrl = ref('')
const isActive = ref(true)
const metadataText = ref('')
const errors = ref<FormErrors>({})
const formError = ref('')
const submitting = ref(false)

const isEdit = computed(() => props.room !== null)
const modalTitle = computed(() => (isEdit.value ? '编辑房间' : '新建房间'))

function resetForm() {
  const room = props.room
  areaId.value = room?.area_id ?? props.areas[0]?.id ?? ''
  code.value = room?.code ?? ''
  name.value = room?.name ?? ''
  capacity.value = String(room?.capacity ?? 0)
  deviceSlot.value = room?.device?.device_slot ?? room?.device_slot ?? ''
  webhookUrl.value = room?.webhook_url ?? ''
  isActive.value = room?.is_active ?? true
  const metadata = room?.metadata ?? {}
  metadataText.value = Object.keys(metadata).length ? JSON.stringify(metadata, null, 2) : ''
  errors.value = {}
  formError.value = ''
}

watch(
  () => props.open,
  (open) => {
    if (open) resetForm()
  },
)

function parseMetadata(): { value: Record<string, unknown> | null; error: string } {
  const trimmed = metadataText.value.trim()
  if (!trimmed) return { value: null, error: '' }
  try {
    const parsed: unknown = JSON.parse(trimmed)
    if (parsed === null) return { value: null, error: '' }
    if (typeof parsed !== 'object' || Array.isArray(parsed)) {
      return { value: null, error: 'metadata 必须是 JSON 对象' }
    }
    return { value: parsed as Record<string, unknown>, error: '' }
  } catch {
    return { value: null, error: 'metadata 不是合法的 JSON' }
  }
}

async function submit() {
  const nextErrors: FormErrors = {}
  if (!areaId.value) nextErrors.area_id = '请选择所属区域'
  if (!code.value.trim()) nextErrors.code = '请输入房间编码'
  if (!name.value.trim()) nextErrors.name = '请输入房间名称'
  const capacityNumber = Number(capacity.value)
  if (!Number.isInteger(capacityNumber) || capacityNumber < 0) {
    nextErrors.capacity = '容量必须是 0 或正整数'
  }
  const parsed = parseMetadata()
  if (parsed.error) nextErrors.metadata = parsed.error
  errors.value = nextErrors
  formError.value = ''
  if (Object.keys(nextErrors).length > 0) return

  const base = {
    area_id: areaId.value,
    code: code.value.trim(),
    name: name.value.trim(),
    capacity: capacityNumber,
    device_slot: deviceSlot.value.trim() || null,
    webhook_url: webhookUrl.value.trim() || null,
    is_active: isActive.value,
  }

  submitting.value = true
  try {
    if (props.room) {
      const payload: UpdateRoomRequest = { ...base, metadata: parsed.value }
      const saved = await api.updateRoom(props.room.id, payload)
      toast.success('房间已更新', saved.name)
      emit('saved', saved)
    } else {
      const payload: CreateRoomRequest = { ...base, metadata: parsed.value ?? undefined }
      const saved = await api.createRoom(payload)
      toast.success('房间已创建', saved.name)
      emit('saved', saved)
    }
  } catch (caught) {
    if (isForbidden(caught)) {
      formError.value = '缺少权限：当前账号没有房间管理权限（room:manage）。'
    } else {
      formError.value = errorMessage(caught, '保存房间失败')
    }
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal :open="open" :title="modalTitle" size="lg" @close="emit('close')">
    <form class="space-y-3.5" novalidate @submit.prevent="submit">
      <div
        v-if="formError"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] leading-5 text-rose-700"
      >
        {{ formError }}
      </div>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="所属区域" input-id="room-area" required :error="errors.area_id">
          <select id="room-area" v-model="areaId" data-autofocus class="select">
            <option value="">请选择区域</option>
            <option v-for="area in areas" :key="area.id" :value="area.id">
              {{ area.name }} ({{ area.code }})
            </option>
          </select>
        </FormField>
        <FormField
          label="房间编码"
          input-id="room-code"
          required
          :error="errors.code"
          hint="例如 room-501"
        >
          <input id="room-code" v-model="code" class="input font-mono" placeholder="room-501" />
        </FormField>
      </div>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="房间名称" input-id="room-name" required :error="errors.name">
          <input id="room-name" v-model="name" class="input" placeholder="501 教室" />
        </FormField>
        <FormField label="容量" input-id="room-capacity" required :error="errors.capacity">
          <input
            id="room-capacity"
            v-model="capacity"
            type="number"
            min="0"
            step="1"
            class="input"
          />
        </FormField>
      </div>

      <div class="grid grid-cols-2 gap-3">
        <FormField
          label="设备槽位"
          input-id="room-slot"
          hint="留空表示不自动分配；也可在房间详情里绑定设备"
        >
          <input
            id="room-slot"
            v-model="deviceSlot"
            class="input font-mono"
            placeholder="room-501"
          />
        </FormField>
        <FormField label="Webhook 地址" input-id="room-webhook" hint="房间级通知地址，留空不推送">
          <input
            id="room-webhook"
            v-model="webhookUrl"
            class="input font-mono"
            placeholder="https://example.com/hooks/room"
          />
        </FormField>
      </div>

      <label class="flex items-center gap-2 text-[11.5px] text-ink-soft">
        <input
          v-model="isActive"
          type="checkbox"
          class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
        />
        启用该房间
      </label>

      <FormField
        label="metadata（JSON 对象）"
        input-id="room-metadata"
        :error="errors.metadata"
        hint='可选，例如 {"floor":5,"tags":["教室"]}'
      >
        <textarea
          id="room-metadata"
          v-model="metadataText"
          class="textarea font-mono"
          spellcheck="false"
          placeholder="{}"
        />
      </FormField>
    </form>

    <template #footer>
      <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
      <AppButton variant="primary" :loading="submitting" @click="submit">
        {{ isEdit ? '保存修改' : '创建房间' }}
      </AppButton>
    </template>
  </AppModal>
</template>
