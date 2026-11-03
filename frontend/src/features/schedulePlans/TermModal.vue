<script setup lang="ts">
import { reactive, ref, watch } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'

import type { Term, TermInput } from './types'

const props = withDefaults(
  defineProps<{
    open: boolean
    mode: 'create' | 'edit'
    term?: Term | null
    saving?: boolean
    error?: string
  }>(),
  { term: null, saving: false, error: '' },
)

const emit = defineEmits<{
  close: []
  submit: [value: TermInput]
}>()

const form = reactive<Required<TermInput>>({
  code: '',
  name: '',
  start_date: '',
  end_date: '',
  is_active: true,
})
const validationError = ref('')

watch(
  () => props.open,
  (open) => {
    if (!open) return
    validationError.value = ''
    if (props.mode === 'edit' && props.term) {
      Object.assign(form, {
        code: props.term.code,
        name: props.term.name,
        start_date: props.term.start_date,
        end_date: props.term.end_date,
        is_active: props.term.is_active,
      })
    } else {
      Object.assign(form, {
        code: '',
        name: '',
        start_date: '',
        end_date: '',
        is_active: true,
      })
    }
  },
)

function submit() {
  validationError.value = ''
  if (!form.code.trim()) {
    validationError.value = '请填写学期编码'
    return
  }
  if (!form.name.trim()) {
    validationError.value = '请填写学期名称'
    return
  }
  if (!/^\d{4}-\d{2}-\d{2}$/.test(form.start_date) || !/^\d{4}-\d{2}-\d{2}$/.test(form.end_date)) {
    validationError.value = '请选择开始与结束日期'
    return
  }
  if (form.start_date > form.end_date) {
    validationError.value = '开始日期不能晚于结束日期'
    return
  }
  emit('submit', {
    code: form.code.trim(),
    name: form.name.trim(),
    start_date: form.start_date,
    end_date: form.end_date,
    is_active: form.is_active,
  })
}
</script>

<template>
  <AppModal
    :open="open"
    :title="mode === 'create' ? '新建学期' : '编辑学期'"
    description="学期用于归集排课计划；默认选中包含今天的启用学期。"
    size="md"
    @close="emit('close')"
  >
    <form id="term-form" class="space-y-3.5" @submit.prevent="submit">
      <div class="grid grid-cols-2 gap-3">
        <FormField label="学期编码" required input-id="term-code" hint="例如 2026-fall">
          <input
            id="term-code"
            v-model="form.code"
            class="input"
            maxlength="40"
            placeholder="2026-fall"
            data-autofocus
          />
        </FormField>
        <FormField label="学期名称" required input-id="term-name">
          <input
            id="term-name"
            v-model="form.name"
            class="input"
            maxlength="80"
            placeholder="2026 秋季学期"
          />
        </FormField>
      </div>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="开始日期" required input-id="term-start">
          <input id="term-start" v-model="form.start_date" type="date" class="input" />
        </FormField>
        <FormField label="结束日期" required input-id="term-end">
          <input id="term-end" v-model="form.end_date" type="date" class="input" />
        </FormField>
      </div>

      <div
        class="flex items-center justify-between gap-3 rounded-md border border-line bg-canvas/40 px-3 py-2.5"
      >
        <div>
          <p class="text-[12px] font-medium text-ink">启用学期</p>
          <p class="mt-0.5 text-[10.5px] text-faint">停用后仍保留历史计划，但不再作为默认学期。</p>
        </div>
        <ToggleSwitch v-model="form.is_active" />
      </div>

      <div
        v-if="validationError || error"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
      >
        {{ validationError || error }}
      </div>
    </form>

    <template #footer>
      <AppButton :disabled="saving" @click="emit('close')">取消</AppButton>
      <AppButton type="submit" form="term-form" variant="primary" :loading="saving" icon="check">
        {{ mode === 'create' ? '创建学期' : '保存修改' }}
      </AppButton>
    </template>
  </AppModal>
</template>
