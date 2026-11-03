<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'

import type { MasterDataType, MasterFormValue, MasterRecord } from './types'
import { emptyMasterForm, masterFormOf, masterTypeLabel, validateMasterForm } from './types'

const props = withDefaults(
  defineProps<{
    open: boolean
    type: MasterDataType
    record?: MasterRecord | null
    saving?: boolean
    error?: string
  }>(),
  { record: null, saving: false, error: '' },
)

const emit = defineEmits<{
  close: []
  submit: [value: MasterFormValue]
}>()

const form = reactive<MasterFormValue>(emptyMasterForm('course'))
const validationError = ref('')

const mode = computed(() => (props.record ? 'edit' : 'create'))
const label = computed(() => masterTypeLabel(props.type))

const title = computed(() => `${mode.value === 'create' ? '新建' : '编辑'}${label.value}`)

const description = computed(() => {
  if (props.type === 'teacher') return '教师用于计划条目指派；可留空联系方式。'
  if (props.type === 'class') return '班级是计划条目的可选维度，发布后暂不写入事件 payload。'
  return '课程被计划条目引用；删除前需确认没有被引用。'
})

watch(
  () => [props.open, props.type, props.record] as const,
  ([open]) => {
    if (!open) return
    validationError.value = ''
    Object.assign(
      form,
      props.record ? masterFormOf(props.type, props.record) : emptyMasterForm(props.type),
    )
  },
  { immediate: true },
)

function submit() {
  const message = validateMasterForm(props.type, form)
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
    <form id="master-data-form" class="space-y-3.5" @submit.prevent="submit">
      <div class="grid grid-cols-2 gap-3">
        <FormField label="编码" required input-id="master-code" hint="唯一，保存后复用">
          <input
            id="master-code"
            v-model="form.code"
            class="input"
            maxlength="40"
            :placeholder="
              type === 'course' ? 'MATH101' : type === 'teacher' ? 'T-zhangwei' : 'CS-2601'
            "
            data-autofocus
          />
        </FormField>
        <FormField label="名称" required input-id="master-name">
          <input
            id="master-name"
            v-model="form.name"
            class="input"
            maxlength="80"
            :placeholder="
              type === 'course' ? '高等数学' : type === 'teacher' ? '张伟' : '计算机 2601 班'
            "
          />
        </FormField>
      </div>

      <FormField
        v-if="type === 'course'"
        label="课程颜色"
        input-id="master-color"
        hint="排课计划中用于区分课程；留空使用默认色。"
      >
        <div class="flex items-center gap-2">
          <input
            id="master-color"
            v-model="form.color"
            class="input"
            maxlength="20"
            placeholder="#3a5bd9"
          />
          <span
            class="h-8 w-8 shrink-0 rounded-md border border-line"
            :style="{ background: form.color || '#eef2ff' }"
          />
        </div>
      </FormField>

      <div v-if="type === 'teacher'" class="grid grid-cols-2 gap-3">
        <FormField label="手机号" input-id="master-phone">
          <input
            id="master-phone"
            v-model="form.phone"
            class="input"
            maxlength="30"
            placeholder="选填"
          />
        </FormField>
        <FormField label="邮箱" input-id="master-email">
          <input
            id="master-email"
            v-model="form.email"
            class="input"
            maxlength="120"
            placeholder="选填"
          />
        </FormField>
      </div>

      <FormField
        v-if="type === 'class'"
        label="年级"
        input-id="master-grade"
        hint="用于列表筛选与展示，例如 2026 级。"
      >
        <input
          id="master-grade"
          v-model="form.grade"
          class="input"
          maxlength="40"
          placeholder="选填"
        />
      </FormField>

      <div
        class="flex items-center justify-between gap-3 rounded-md border border-line bg-canvas/40 px-3 py-2.5"
      >
        <div>
          <p class="text-[12px] font-medium text-ink">启用</p>
          <p class="mt-0.5 text-[10.5px] text-faint">停用后不能在新的计划条目中选择。</p>
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
      <AppButton
        type="submit"
        form="master-data-form"
        variant="primary"
        :loading="saving"
        icon="check"
      >
        {{ mode === 'create' ? `创建${label}` : '保存修改' }}
      </AppButton>
    </template>
  </AppModal>
</template>
