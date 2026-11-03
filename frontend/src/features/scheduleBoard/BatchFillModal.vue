<script setup lang="ts">
/**
 * C 矩阵框选后的「批量应用课程」弹窗。
 *
 * 表单：课程名称 / 教师 / 备注 + 作用模式（仅填充空格 / 覆盖已有课）。
 * 预览由 `planBatchFill` 纯函数实时计算：将新建 X 节、跳过 Y 节、涉及 Z 张课表；
 * 同一天时段重叠 / 超 100 条时展示错误并禁用提交。真正落库由页面在 ensure + load 后重新计划执行。
 */
import { computed, reactive, watch } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'

import type { BoardPayload } from './boardTypes'
import {
  planBatchFill,
  type BatchFillMode,
  type BatchFillValue,
  type MatrixCellRef,
} from './boardActions'

const props = withDefaults(
  defineProps<{
    open: boolean
    payload: BoardPayload | null
    cells: MatrixCellRef[]
    saving?: boolean
    error?: string
  }>(),
  { saving: false, error: '' },
)

const emit = defineEmits<{
  close: []
  submit: [payload: BatchFillValue & { mode: BatchFillMode }]
}>()

const form = reactive({
  course_name: '',
  teacher: '',
  note: '',
  mode: 'empty' as BatchFillMode,
})

watch(
  () => props.open,
  (open) => {
    if (!open) return
    form.course_name = ''
    form.teacher = ''
    form.note = ''
    form.mode = 'empty'
  },
)

const preview = computed(() => planBatchFill(props.payload, props.cells, form, form.mode))
const roomCount = computed(() => new Set(props.cells.map((cell) => cell.room_id)).size)
const scheduleCount = computed(() => preview.value.perRoom.length)
const canSubmit = computed(
  () => Boolean(form.course_name.trim()) && preview.value.errors.length === 0 && !props.saving,
)

function submit() {
  if (!canSubmit.value) return
  emit('submit', {
    course_name: form.course_name.trim(),
    teacher: form.teacher.trim(),
    note: form.note.trim(),
    mode: form.mode,
  })
}
</script>

<template>
  <AppModal
    :open="open"
    title="批量应用课程"
    description="对选中的矩阵格子批量写入周模板；时段取对应节次的固定时间。"
    size="md"
    @close="emit('close')"
  >
    <form id="batch-fill-form" class="space-y-3.5" @submit.prevent="submit">
      <div
        class="flex items-center justify-between rounded-lg border border-line bg-canvas/60 px-3 py-2 text-[11px] text-ink-soft"
      >
        <span
          >已选 <b class="tabular-nums text-ink">{{ cells.length }}</b> 格（跨
          {{ roomCount }} 间教室）</span
        >
        <span class="text-faint">Esc 取消选择</span>
      </div>

      <FormField label="课程名称" required input-id="batch-course-name">
        <input
          id="batch-course-name"
          v-model="form.course_name"
          class="input"
          maxlength="120"
          placeholder="例如：体育"
          data-autofocus
        />
      </FormField>

      <FormField label="教师" input-id="batch-teacher">
        <input
          id="batch-teacher"
          v-model="form.teacher"
          class="input"
          maxlength="80"
          placeholder="选填"
        />
      </FormField>

      <div>
        <p class="mb-1.5 text-[11.5px] font-medium text-ink-soft">填充方式</p>
        <div class="grid grid-cols-2 gap-2">
          <label
            class="cursor-pointer rounded-lg border px-2.5 py-2 transition-colors"
            :class="
              form.mode === 'empty'
                ? 'border-accent bg-accent-soft/40'
                : 'border-line hover:bg-canvas/60'
            "
          >
            <input v-model="form.mode" type="radio" value="empty" class="sr-only" />
            <span class="text-[11.5px] font-medium text-ink">仅填充空格</span>
            <span class="mt-0.5 block text-[10px] leading-4 text-muted">
              跳过已有课程的格子（默认）
            </span>
          </label>
          <label
            class="cursor-pointer rounded-lg border px-2.5 py-2 transition-colors"
            :class="
              form.mode === 'overwrite'
                ? 'border-accent bg-accent-soft/40'
                : 'border-line hover:bg-canvas/60'
            "
          >
            <input v-model="form.mode" type="radio" value="overwrite" class="sr-only" />
            <span class="text-[11.5px] font-medium text-ink">覆盖已有课</span>
            <span class="mt-0.5 block text-[10px] leading-4 text-muted">
              同一格已有课程会被替换
            </span>
          </label>
        </div>
      </div>

      <FormField label="备注" input-id="batch-note">
        <textarea
          id="batch-note"
          v-model="form.note"
          class="textarea min-h-[56px]"
          maxlength="500"
          placeholder="选填，会写入每条新课程"
        />
      </FormField>

      <div class="rounded-lg border border-line bg-canvas/50 px-3 py-2 text-[11px] leading-5">
        <p class="text-ink-soft">
          将新建 <b class="tabular-nums text-accent">{{ preview.created }}</b> 节 · 跳过
          <b class="tabular-nums">{{ preview.skipped }}</b> 节 · 涉及
          <b class="tabular-nums">{{ scheduleCount }}</b> 张课表
        </p>
        <p v-if="preview.errors.length" class="mt-1 text-rose-600">
          {{ preview.errors.join('；') }}
        </p>
        <p
          v-else-if="form.mode === 'empty' && preview.created === 0 && preview.skipped > 0"
          class="mt-1 text-amber-600"
        >
          所选格子都已有课程，可切换「覆盖已有课」或调整选区。
        </p>
        <p v-else-if="preview.created === 0" class="mt-1 text-amber-600">
          当前选区没有可写入的格子。
        </p>
        <p v-else class="mt-1 text-[10px] text-faint">
          执行时先为缺课表的教室补默认课表，再按课表全量替换并重建未来实例。
        </p>
      </div>

      <p
        v-if="error"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
      >
        {{ error }}
      </p>
    </form>

    <template #footer>
      <AppButton :disabled="saving" @click="emit('close')">取消</AppButton>
      <AppButton
        type="submit"
        form="batch-fill-form"
        variant="primary"
        icon="check"
        :loading="saving"
        :disabled="!canSubmit"
      >
        批量应用
      </AppButton>
    </template>
  </AppModal>
</template>
