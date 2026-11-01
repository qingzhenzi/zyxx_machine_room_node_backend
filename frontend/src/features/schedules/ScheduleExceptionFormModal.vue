<script setup lang="ts">
import { reactive, ref, watch } from 'vue'

import type { ScheduleException, UpsertExceptionRequest } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { today } from '@/utils/format'

import type { SlotDraft } from './scheduleForm'
import { emptySlotDraft, toNewExceptionSlots, validateSlotDrafts } from './scheduleForm'

const props = withDefaults(
  defineProps<{
    open: boolean
    exception?: ScheduleException | null
    saving?: boolean
  }>(),
  { exception: null, saving: false },
)

const emit = defineEmits<{
  close: []
  submit: [payload: { date: string; input: UpsertExceptionRequest }]
}>()

const form = reactive<{ date: string; kind: 'cancel' | 'replace'; note: string }>({
  date: today(),
  kind: 'cancel',
  note: '',
})
const rows = ref<SlotDraft[]>([])
const error = ref('')

function slotDraft(exception: ScheduleException): SlotDraft[] {
  return (exception.slots ?? []).map((slot) => ({
    day_of_week: 1,
    start_time: slot.start_time ?? '',
    end_time: slot.end_time ?? '',
    course_name: slot.course_name ?? '',
    teacher: slot.teacher ?? '',
    note: slot.note ?? '',
  }))
}

watch(
  () => props.open,
  (open) => {
    if (!open) return
    error.value = ''
    const source = props.exception
    if (source) {
      form.date = source.exception_date ?? today()
      form.kind = source.kind ?? 'cancel'
      form.note = source.note ?? ''
      rows.value = slotDraft(source)
    } else {
      form.date = today()
      form.kind = 'cancel'
      form.note = ''
      rows.value = []
    }
  },
)

function addRow() {
  rows.value.push(emptySlotDraft())
  error.value = ''
}

function removeRow(index: number) {
  rows.value.splice(index, 1)
  error.value = ''
}

function submit() {
  if (!form.date) {
    error.value = '请选择覆盖日期'
    return
  }
  if (form.kind === 'replace') {
    if (!rows.value.length) {
      error.value = '替换当天至少需要一条时段'
      return
    }
    const message = validateSlotDrafts(rows.value, { requireDay: false })
    if (message) {
      error.value = message
      return
    }
  }
  error.value = ''
  emit('submit', {
    date: form.date,
    input: {
      kind: form.kind,
      note: form.note.trim(),
      slots: form.kind === 'replace' ? toNewExceptionSlots(rows.value) : [],
    },
  })
}
</script>

<template>
  <AppModal
    :open="open"
    :title="exception ? '编辑单日覆盖' : '新增单日覆盖'"
    description="单日覆盖优先于周模板，保存后会重建未来的排课实例。"
    size="lg"
    @close="emit('close')"
  >
    <form class="space-y-3.5" @submit.prevent="submit">
      <div class="grid grid-cols-2 gap-3">
        <FormField
          label="覆盖日期"
          required
          input-id="exception-date"
          :hint="exception ? '覆盖日期不可修改，如需调整请删除后重新添加。' : ''"
        >
          <input
            id="exception-date"
            v-model="form.date"
            type="date"
            class="input"
            :disabled="!!exception"
            data-autofocus
          />
        </FormField>
        <FormField label="覆盖类型" required input-id="exception-kind">
          <select id="exception-kind" v-model="form.kind" class="select">
            <option value="cancel">停课（取消当天全部时段）</option>
            <option value="replace">调课（替换当天时段）</option>
          </select>
        </FormField>
      </div>

      <FormField label="备注" input-id="exception-note" hint="可选，例如：国庆假期、临时调课。">
        <textarea
          id="exception-note"
          v-model="form.note"
          class="textarea"
          maxlength="500"
          placeholder="填写停课/调课原因"
        />
      </FormField>

      <div v-if="form.kind === 'replace'" class="space-y-2">
        <div class="flex items-center justify-between">
          <p class="text-[11.5px] font-medium text-ink-soft">替换时段</p>
          <AppButton size="sm" icon="plus" @click="addRow">添加时段</AppButton>
        </div>

        <div
          v-for="(row, index) in rows"
          :key="index"
          class="rounded-md border border-line bg-canvas/40 px-2.5 py-2.5"
        >
          <div class="grid grid-cols-[108px_108px_minmax(0,1fr)_108px_28px] items-center gap-2">
            <input
              v-model="row.start_time"
              type="time"
              class="input input-sm"
              :aria-label="`第 ${index + 1} 个时段开始时间`"
            />
            <input
              v-model="row.end_time"
              type="time"
              class="input input-sm"
              :aria-label="`第 ${index + 1} 个时段结束时间`"
            />
            <input
              v-model="row.course_name"
              class="input input-sm"
              maxlength="80"
              placeholder="课程名称"
              :aria-label="`第 ${index + 1} 个时段课程名称`"
            />
            <input
              v-model="row.teacher"
              class="input input-sm"
              maxlength="80"
              placeholder="教师"
              :aria-label="`第 ${index + 1} 个时段教师`"
            />
            <IconButton
              icon="trash"
              label="删除时段"
              size="sm"
              :disabled="saving"
              @click="removeRow(index)"
            />
          </div>
          <input
            v-model="row.note"
            class="input input-sm mt-2"
            maxlength="200"
            placeholder="备注（可选）"
            :aria-label="`第 ${index + 1} 个时段备注`"
          />
        </div>

        <p v-if="!rows.length" class="text-[10.5px] text-faint">
          调课需要至少一条替换时段，房间沿用课表所属房间。
        </p>
        <p v-else class="text-[10.5px] text-faint">替换时段的房间沿用课表所属房间。</p>
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
        {{ exception ? '保存覆盖' : '创建覆盖' }}
      </AppButton>
    </template>
  </AppModal>
</template>
