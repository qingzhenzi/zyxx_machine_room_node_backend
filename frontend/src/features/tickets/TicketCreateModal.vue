<script setup lang="ts">
/**
 * 新建工单弹窗：标题必填，可选分类 / 优先级 / 关联房间 / 设备序列号 / 问题描述。
 * 提交成功后由页面跳转到详情页。
 */
import { ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type { CreateTicketRequest, TicketCategory, TicketPriority } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'

import {
  TICKET_CATEGORY_ORDER,
  TICKET_PRIORITY_ORDER,
  ticketCategoryMeta,
  ticketPriorityMeta,
} from './ticketMeta'

interface RoomOption {
  id?: string | null
  name?: string | null
  code?: string | null
}

const props = withDefaults(
  defineProps<{
    open: boolean
    rooms?: RoomOption[]
    /** 从房间 / 设备 / 机器详情「报修」入口带入的预填值（P1 Wave 2）。 */
    initialRoomId?: string
    initialSerial?: string
    initialDeviceSlot?: string
    initialTitle?: string
  }>(),
  {
    rooms: () => [],
    initialRoomId: '',
    initialSerial: '',
    initialDeviceSlot: '',
    initialTitle: '',
  },
)

const emit = defineEmits<{
  close: []
  created: [ticketId: string]
}>()

const toast = useToastStore()

const title = ref('')
const category = ref<TicketCategory>('other')
const priority = ref<TicketPriority>('normal')
const roomId = ref('')
const serial = ref('')
const deviceSlot = ref('')
const description = ref('')
const titleError = ref('')
const formError = ref('')
const submitting = ref(false)

function resetForm() {
  title.value = props.initialTitle || ''
  category.value = 'other'
  priority.value = 'normal'
  roomId.value = props.initialRoomId || ''
  serial.value = props.initialSerial || ''
  deviceSlot.value = props.initialDeviceSlot || ''
  description.value = ''
  titleError.value = ''
  formError.value = ''
}

watch(
  () => props.open,
  (open) => {
    if (open) resetForm()
  },
)

async function submit() {
  titleError.value = ''
  formError.value = ''
  if (!title.value.trim()) {
    titleError.value = '请输入工单标题'
    return
  }

  const payload: CreateTicketRequest = {
    title: title.value.trim(),
    description: description.value.trim() || undefined,
    category: category.value,
    priority: priority.value,
    room_id: roomId.value || undefined,
    serial: serial.value.trim() || undefined,
    device_slot: deviceSlot.value.trim() || undefined,
  }

  submitting.value = true
  try {
    const ticket = await api.createTicket(payload)
    toast.success('工单已创建', ticket.code)
    emit('created', ticket.id)
  } catch (caught) {
    if (isForbidden(caught)) {
      formError.value = '缺少权限：当前账号没有创建工单权限（ticket:create）。'
    } else {
      formError.value = errorMessage(caught, '创建工单失败，请稍后重试')
    }
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="props.open"
    title="新建工单"
    description="手工上报问题；机器健康、设备离线等自动工单由系统规则创建。"
    size="lg"
    @close="emit('close')"
  >
    <form class="flex flex-col gap-4" @submit.prevent="submit">
      <FormField label="标题" input-id="ticket-title" required :error="titleError">
        <input
          id="ticket-title"
          v-model="title"
          class="input"
          maxlength="200"
          placeholder="一句话说明问题，例如：room-101 主机 GPU 温度过高"
          data-autofocus
        />
      </FormField>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="分类" input-id="ticket-category">
          <select id="ticket-category" v-model="category" class="select">
            <option v-for="key in TICKET_CATEGORY_ORDER" :key="key" :value="key">
              {{ ticketCategoryMeta(key).label }}
            </option>
          </select>
        </FormField>
        <FormField label="优先级" input-id="ticket-priority">
          <select id="ticket-priority" v-model="priority" class="select">
            <option v-for="key in TICKET_PRIORITY_ORDER" :key="key" :value="key">
              {{ ticketPriorityMeta(key).label }}（SLA {{ ticketPriorityMeta(key).slaHours }} 小时）
            </option>
          </select>
        </FormField>
      </div>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="关联房间" input-id="ticket-room" hint="可选，用于把工单挂到具体房间">
          <select id="ticket-room" v-model="roomId" class="select">
            <option value="">不关联房间</option>
            <option
              v-for="room in props.rooms"
              :key="room.id ?? room.code ?? ''"
              :value="room.id ?? ''"
            >
              {{ room.name ?? room.code ?? room.id }}
            </option>
          </select>
        </FormField>
        <FormField
          label="设备槽位"
          input-id="ticket-device-slot"
          hint="可选，例如 中控主机 / 电子班牌"
        >
          <input
            id="ticket-device-slot"
            v-model="deviceSlot"
            class="input"
            maxlength="100"
            placeholder="例如 中控主机"
          />
        </FormField>
      </div>

      <FormField
        label="设备序列号"
        input-id="ticket-serial"
        hint="可选，填写后可跳转设备 / 机器详情"
      >
        <input
          id="ticket-serial"
          v-model="serial"
          class="input font-mono"
          placeholder="例如 4f40ccef2a7b3d21"
        />
      </FormField>

      <FormField label="问题描述" input-id="ticket-description">
        <textarea
          id="ticket-description"
          v-model="description"
          class="textarea min-h-[110px]"
          placeholder="现象、影响范围、已尝试的排查步骤…"
        />
      </FormField>

      <p
        v-if="formError"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
      >
        {{ formError }}
      </p>

      <div class="flex items-center justify-end gap-2 border-t border-line pt-4">
        <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
        <AppButton type="submit" variant="primary" icon="plus" :loading="submitting">
          创建工单
        </AppButton>
      </div>
    </form>
  </AppModal>
</template>
