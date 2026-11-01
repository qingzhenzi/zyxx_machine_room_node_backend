<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { api } from '@/api'
import type { Permission, RoleSummary } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
import PermissionPicker from '@/features/roles/PermissionPicker.vue'
import { errorMessage } from '@/utils/errors'

const props = defineProps<{
  open: boolean
  role: RoleSummary | null
  permissions: Permission[]
}>()

const emit = defineEmits<{
  close: []
  saved: [role: RoleSummary]
}>()

const selected = ref<string[]>([])
const submitting = ref(false)
const error = ref('')

const readonly = computed(() => props.role?.is_system === true)
const roleLabel = computed(() => props.role?.name || props.role?.code || '角色')

watch(
  () => props.open,
  (open) => {
    if (!open) return
    selected.value = [...(props.role?.permissions ?? [])]
    submitting.value = false
    error.value = ''
  },
)

async function submit() {
  if (readonly.value || !props.role?.id) return
  error.value = ''
  submitting.value = true
  try {
    emit('saved', await api.setRolePermissions(props.role.id, [...selected.value]))
  } catch (caught) {
    error.value = errorMessage(caught, '保存角色权限失败')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="open"
    :title="readonly ? '查看角色权限' : '编辑角色权限'"
    :description="`${roleLabel}（${role?.code ?? '—'}）的权限点配置。`"
    size="lg"
    @close="emit('close')"
  >
    <div
      v-if="error"
      role="alert"
      class="mb-4 flex items-start gap-2 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] leading-5 text-rose-700"
    >
      <AppIcon name="alert" :size="13" class="mt-0.5 shrink-0" />
      <span>{{ error }}</span>
    </div>

    <div
      v-if="readonly"
      class="mb-4 flex items-start gap-2 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-[10.5px] leading-5 text-amber-700"
    >
      <AppIcon name="lock" :size="13" class="mt-0.5 shrink-0" />
      <span>系统内置角色的权限只读，后端会返回 409（系统内置角色的权限不可修改）。</span>
    </div>

    <PermissionPicker v-model="selected" :permissions="permissions" :disabled="readonly" />

    <template #footer>
      <AppButton :disabled="submitting" @click="emit('close')">
        {{ readonly ? '关闭' : '取消' }}
      </AppButton>
      <AppButton v-if="!readonly" variant="primary" :loading="submitting" @click="submit">
        保存权限
      </AppButton>
    </template>
  </AppModal>
</template>
