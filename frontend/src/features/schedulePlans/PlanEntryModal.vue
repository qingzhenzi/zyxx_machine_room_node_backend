<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'

import type { Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'

import type { Course, PlanEntryFormValue, SchoolClass, Teacher } from './types'
import { emptyEntryForm, minutesOf, validateEntryForm } from './types'

const props = withDefaults(
  defineProps<{
    open: boolean
    mode: 'create' | 'edit'
    rooms: Room[]
    courses: Course[]
    teachers: Teacher[]
    classes: SchoolClass[]
    initial?: PlanEntryFormValue | null
    error?: string
  }>(),
  { initial: null, error: '' },
)

const emit = defineEmits<{
  close: []
  submit: [value: PlanEntryFormValue]
}>()

const form = reactive<PlanEntryFormValue>(emptyEntryForm())
const validationError = ref('')

const roomGroups = computed(() => {
  const groups = new Map<string, Room[]>()
  const sorted = [...props.rooms].sort(
    (left, right) =>
      left.area_name.localeCompare(right.area_name, 'zh-CN') ||
      left.code.localeCompare(right.code, 'zh-CN'),
  )
  for (const room of sorted) {
    const list = groups.get(room.area_name) ?? []
    list.push(room)
    groups.set(room.area_name, list)
  }
  return [...groups.entries()].map(([area, rooms]) => ({ area, rooms }))
})

const taskDuration = computed(() => {
  if (!form.start_time || !form.end_time) return 0
  const minutes = minutesOf(form.end_time) - minutesOf(form.start_time)
  return minutes > 0 ? minutes : 0
})

watch(
  () => props.open,
  (open) => {
    if (!open) return
    validationError.value = ''
    Object.assign(form, props.initial ?? emptyEntryForm())
  },
)

function submit() {
  const message = validateEntryForm(form)
  if (message) {
    validationError.value = message
    return
  }
  validationError.value = ''
  emit('submit', { ...form })
}
</script>

<template>
  <AppModal
    :open="open"
    :title="mode === 'create' ? '新建条目' : '编辑条目'"
    description="条目按「模板周」维护：房间 + 星期 + 时段；同一房间同一天不能时段重叠。"
    size="lg"
    @close="emit('close')"
  >
    <form id="plan-entry-form" class="space-y-3.5" @submit.prevent="submit">
      <div class="grid grid-cols-2 gap-3">
        <FormField label="房间" required input-id="entry-room">
          <select id="entry-room" v-model="form.room_id" class="select" data-autofocus>
            <option value="">请选择房间</option>
            <optgroup v-for="group in roomGroups" :key="group.area" :label="group.area">
              <option v-for="room in group.rooms" :key="room.id" :value="room.id">
                {{ room.code }} · {{ room.name }}
              </option>
            </optgroup>
          </select>
        </FormField>
        <div class="grid grid-cols-3 gap-2">
          <FormField label="星期" required input-id="entry-day">
            <select id="entry-day" v-model.number="form.day_of_week" class="select">
              <option v-for="day in 7" :key="day" :value="day">
                周{{ '一二三四五六日'[day - 1] }}
              </option>
            </select>
          </FormField>
          <FormField label="开始" required input-id="entry-start">
            <input id="entry-start" v-model="form.start_time" type="time" step="60" class="input" />
          </FormField>
          <FormField label="结束" required input-id="entry-end">
            <input id="entry-end" v-model="form.end_time" type="time" step="60" class="input" />
          </FormField>
        </div>
      </div>

      <div class="grid grid-cols-3 gap-3">
        <FormField label="课程" required input-id="entry-course">
          <select id="entry-course" v-model="form.course_id" class="select">
            <option value="">请选择课程</option>
            <option v-for="course in courses" :key="course.id" :value="course.id">
              {{ course.name }}{{ course.is_active ? '' : '（已停用）' }}
            </option>
          </select>
        </FormField>
        <FormField label="教师" input-id="entry-teacher">
          <select id="entry-teacher" v-model="form.teacher_id" class="select">
            <option value="">未指定</option>
            <option v-for="teacher in teachers" :key="teacher.id" :value="teacher.id">
              {{ teacher.name }}{{ teacher.is_active ? '' : '（已停用）' }}
            </option>
          </select>
        </FormField>
        <FormField label="班级" input-id="entry-class">
          <select id="entry-class" v-model="form.class_id" class="select">
            <option value="">未指定</option>
            <option v-for="item in classes" :key="item.id" :value="item.id">
              {{ item.name }}{{ item.is_active ? '' : '（已停用）' }}
            </option>
          </select>
        </FormField>
      </div>

      <FormField
        label="备注"
        input-id="entry-note"
        :hint="
          taskDuration
            ? `时长 ${taskDuration} 分钟；备注不会写入事件 payload。`
            : '会随条目保存并展示。'
        "
      >
        <textarea
          id="entry-note"
          v-model="form.note"
          class="textarea min-h-[60px]"
          maxlength="200"
          placeholder="例如：机房课需要提前开机"
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
      <AppButton @click="emit('close')">取消</AppButton>
      <AppButton type="submit" form="plan-entry-form" variant="primary" icon="check">
        {{ mode === 'create' ? '添加条目' : '保存条目' }}
      </AppButton>
    </template>
  </AppModal>
</template>
