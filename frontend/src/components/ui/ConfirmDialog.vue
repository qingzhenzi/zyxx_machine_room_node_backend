<script setup lang="ts">
import AppButton from './AppButton.vue'
import AppModal from './AppModal.vue'

withDefaults(
  defineProps<{
    open: boolean
    title: string
    description?: string
    detail?: string
    confirmLabel?: string
    danger?: boolean
    loading?: boolean
  }>(),
  {
    description: '',
    detail: '',
    confirmLabel: '确认',
    danger: false,
    loading: false,
  },
)

const emit = defineEmits<{
  confirm: []
  close: []
}>()
</script>

<template>
  <AppModal :open="open" :title="title" :description="description" size="sm" @close="emit('close')">
    <div
      v-if="detail"
      class="rounded-md border border-line bg-canvas/60 px-3 py-2 text-[11.5px] leading-5 text-ink-soft"
    >
      {{ detail }}
    </div>
    <p v-else class="text-[12px] leading-5 text-muted">该操作不可撤销，请确认后继续。</p>

    <template #footer>
      <AppButton :disabled="loading" @click="emit('close')">取消</AppButton>
      <AppButton
        :variant="danger ? 'danger' : 'primary'"
        :loading="loading"
        @click="emit('confirm')"
      >
        {{ confirmLabel }}
      </AppButton>
    </template>
  </AppModal>
</template>
