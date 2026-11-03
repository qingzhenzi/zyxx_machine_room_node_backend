<script setup lang="ts">
import { ref, watch } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'

const props = withDefaults(
  defineProps<{
    open: boolean
    termName?: string
    saving?: boolean
    error?: string
  }>(),
  { termName: '', saving: false, error: '' },
)

const emit = defineEmits<{
  close: []
  submit: [name: string]
}>()

const name = ref('')
const validationError = ref('')

watch(
  () => props.open,
  (open) => {
    if (!open) return
    name.value = ''
    validationError.value = ''
  },
)

function submit() {
  if (!name.value.trim()) {
    validationError.value = '请填写计划名称'
    return
  }
  validationError.value = ''
  emit('submit', name.value.trim())
}
</script>

<template>
  <AppModal
    :open="open"
    title="新建排课计划"
    :description="`为学期「${termName || '—'}」创建草稿计划；发布后才会写入课表。`"
    size="sm"
    @close="emit('close')"
  >
    <form id="plan-form" class="space-y-3.5" @submit.prevent="submit">
      <FormField
        label="计划名称"
        required
        input-id="plan-name"
        hint="例如「2026 秋季第一版」；克隆时会自动加「副本」。"
      >
        <input
          id="plan-name"
          v-model="name"
          class="input"
          maxlength="80"
          placeholder="2026 秋季第一版"
          data-autofocus
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
      <AppButton type="submit" form="plan-form" variant="primary" :loading="saving" icon="plus">
        创建计划
      </AppButton>
    </template>
  </AppModal>
</template>
