<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'

import { groupRoomsByArea, isoWeekday, weekdayName, type BoardRoom } from './boardTypes'
import { emptySlotForm, validateSlotForm, type BoardSlotFormValue } from './boardActions'

const props = withDefaults(
  defineProps<{
    open: boolean
    mode: 'create' | 'edit'
    rooms: BoardRoom[]
    initial: BoardSlotFormValue | null
    /** `occurrence` = 只影响某一天（写单日覆盖）；默认模板。 */
    scope?: 'template' | 'occurrence'
    /** scope=occurrence 时对应的日期（YYYY-MM-DD）。 */
    scopeDate?: string
    saving?: boolean
    error?: string
  }>(),
  { scope: 'template', scopeDate: '', saving: false, error: '' },
)

const emit = defineEmits<{
  close: []
  submit: [value: BoardSlotFormValue]
}>()

const form = reactive<BoardSlotFormValue>(emptySlotForm())
const validationError = ref('')

const roomGroups = computed(() => groupRoomsByArea(props.rooms))

const isOccurrence = computed(() => props.scope === 'occurrence' && props.mode === 'create')

const scopeWeekday = computed(() =>
  props.scopeDate ? weekdayName(isoWeekday(props.scopeDate)) : '',
)

const title = computed(() => {
  if (props.mode !== 'create') return '编辑课程（周模板）'
  return isOccurrence.value ? '新建本次课程' : '新建课程'
})

const description = computed(() => {
  if (isOccurrence.value) {
    return `仅调整 ${props.scopeDate}（${scopeWeekday.value}）这一天：保存后写入单日覆盖，不影响其它周与周模板。`
  }
  if (props.mode === 'create') {
    return '保存后写入周模板并重建未来排课实例；房间所属课表不存在时会自动补一张默认课表。'
  }
  return '保存后全量替换该课表的周模板并重建；修改房间等于把课程调课到另一间教室。'
})

watch(
  () => props.open,
  (open) => {
    if (!open) return
    validationError.value = ''
    Object.assign(form, props.initial ?? emptySlotForm())
  },
)

function submit() {
  const message = validateSlotForm(form)
  if (message) {
    validationError.value = message
    return
  }
  validationError.value = ''
  emit('submit', { ...form })
}
</script>

<template>
  <AppModal :open="open" :title="title" :description="description" size="md" @close="emit('close')">
    <form id="board-block-form" class="space-y-3.5" @submit.prevent="submit">
      <FormField label="课程名称" required input-id="board-course-name">
        <input
          id="board-course-name"
          v-model="form.course_name"
          class="input"
          maxlength="120"
          placeholder="例如：高等数学"
          data-autofocus
        />
      </FormField>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="教师" input-id="board-teacher">
          <input
            id="board-teacher"
            v-model="form.teacher"
            class="input"
            maxlength="80"
            placeholder="选填"
          />
        </FormField>
        <FormField
          label="房间"
          required
          input-id="board-room"
          :hint="mode === 'edit' ? '改房间 = 调课到另一张课表' : ''"
        >
          <select id="board-room" v-model="form.room_id" class="select">
            <option value="">请选择房间</option>
            <optgroup v-for="group in roomGroups" :key="group.area" :label="group.area">
              <option v-for="room in group.rooms" :key="room.id" :value="room.id">
                {{ room.code }} · {{ room.name }}
              </option>
            </optgroup>
          </select>
        </FormField>
      </div>

      <div class="grid grid-cols-3 gap-3">
        <FormField
          label="星期"
          required
          input-id="board-day"
          :hint="isOccurrence ? '由本次日期决定，不可修改' : ''"
        >
          <select
            v-if="!isOccurrence"
            id="board-day"
            v-model.number="form.day_of_week"
            class="select"
          >
            <option v-for="day in 7" :key="day" :value="day">{{ weekdayName(day) }}</option>
          </select>
          <input
            v-else
            id="board-day"
            class="input bg-canvas text-muted"
            :value="scopeWeekday"
            readonly
            aria-readonly="true"
          />
        </FormField>
        <FormField label="开始时间" required input-id="board-start">
          <input id="board-start" v-model="form.start_time" type="time" step="60" class="input" />
        </FormField>
        <FormField label="结束时间" required input-id="board-end">
          <input id="board-end" v-model="form.end_time" type="time" step="60" class="input" />
        </FormField>
      </div>

      <FormField label="备注" input-id="board-note" hint="会显示在课表详情抽屉里，可留空。">
        <textarea
          id="board-note"
          v-model="form.note"
          class="textarea min-h-[64px]"
          maxlength="500"
          placeholder="例如：机房课需要提前 10 分钟开机"
        />
      </FormField>

      <div
        v-if="validationError || error"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
      >
        {{ validationError || error }}
      </div>
    </form>

    <template #footer>
      <AppButton :disabled="saving" @click="emit('close')">取消</AppButton>
      <AppButton
        type="submit"
        form="board-block-form"
        variant="primary"
        :loading="saving"
        icon="check"
      >
        {{ mode === 'create' ? '保存课程' : '保存修改' }}
      </AppButton>
    </template>
  </AppModal>
</template>
