<script setup lang="ts">
import { reactive, ref, watch } from 'vue'

import type { Room, Schedule } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
import { today } from '@/utils/format'

import type { ScheduleFormValue } from './scheduleForm'
import { emptyScheduleForm, scheduleToForm, validateScheduleForm } from './scheduleForm'

const props = withDefaults(
  defineProps<{
    open: boolean
    mode: 'create' | 'edit'
    schedule?: Schedule | null
    rooms?: Room[]
    saving?: boolean
  }>(),
  { schedule: null, rooms: () => [], saving: false },
)

const emit = defineEmits<{
  close: []
  submit: [value: ScheduleFormValue]
}>()

const form = reactive<ScheduleFormValue>(emptyScheduleForm(today()))
const error = ref('')

const sortedRooms = () =>
  [...props.rooms].sort((left, right) => {
    if (left.is_active !== right.is_active) return left.is_active ? -1 : 1
    return left.name.localeCompare(right.name, 'zh-CN')
  })

watch(
  () => props.open,
  (open) => {
    if (!open) return
    error.value = ''
    const next =
      props.mode === 'edit' && props.schedule
        ? scheduleToForm(props.schedule)
        : emptyScheduleForm(today())
    Object.assign(form, next)
  },
)

function submit() {
  const message = validateScheduleForm(form, props.mode)
  if (message) {
    error.value = message
    return
  }
  error.value = ''
  emit('submit', { ...form, name: form.name.trim() })
}
</script>

<template>
  <AppModal
    :open="open"
    :title="mode === 'create' ? '新建课表' : '编辑课表'"
    :description="
      mode === 'create'
        ? '课表属于某个房间，时区自动跟随房间配置。'
        : '保存后会按新的日期范围重建未来的排课实例。'
    "
    size="lg"
    @close="emit('close')"
  >
    <form class="space-y-3.5" @submit.prevent="submit">
      <FormField
        label="房间"
        required
        input-id="schedule-room"
        :hint="mode === 'create' ? '课表时区与房间时区保持一致。' : '课表创建后不允许更换房间。'"
      >
        <select
          v-if="mode === 'create'"
          id="schedule-room"
          v-model="form.room_id"
          class="select"
          data-autofocus
        >
          <option value="">请选择房间</option>
          <option v-for="room in sortedRooms()" :key="room.id" :value="room.id">
            {{ room.name }}{{ room.is_active ? '' : '（已停用）' }}
          </option>
        </select>
        <input v-else class="input" :value="schedule?.room_name ?? '—'" disabled />
      </FormField>

      <FormField label="课表名称" required input-id="schedule-name">
        <input
          id="schedule-name"
          v-model="form.name"
          class="input"
          maxlength="80"
          placeholder="例如：2026 秋季学期课表"
        />
      </FormField>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="开始日期" required input-id="schedule-start-date">
          <input id="schedule-start-date" v-model="form.start_date" type="date" class="input" />
        </FormField>
        <FormField label="结束日期" required input-id="schedule-end-date">
          <input id="schedule-end-date" v-model="form.end_date" type="date" class="input" />
        </FormField>
      </div>

      <div
        class="flex items-center justify-between gap-3 rounded-md border border-line bg-canvas/40 px-3 py-2.5"
      >
        <div>
          <p class="text-[12px] font-medium text-ink">启用课表</p>
          <p class="mt-0.5 text-[10.5px] text-faint">停用后不再生成新的排课实例，历史实例保留。</p>
        </div>
        <ToggleSwitch v-model="form.is_active" />
      </div>

      <div
        v-if="error"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
      >
        {{ error }}
      </div>
    </form>

    <template #footer>
      <AppButton :disabled="saving" @click="emit('close')">取消</AppButton>
      <AppButton variant="primary" :loading="saving" @click="submit">
        {{ mode === 'create' ? '创建课表' : '保存修改' }}
      </AppButton>
    </template>
  </AppModal>
</template>
