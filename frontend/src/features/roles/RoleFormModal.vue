<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { api } from '@/api'
import type { Permission, RoleSummary } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import PermissionPicker from '@/features/roles/PermissionPicker.vue'
import { errorMessage } from '@/utils/errors'

const props = defineProps<{
  open: boolean
  permissions: Permission[]
}>()

const emit = defineEmits<{
  close: []
  saved: [role: RoleSummary]
}>()

const code = ref('')
const name = ref('')
const description = ref('')
const permissionCodes = ref<string[]>([])
const submitting = ref(false)
const error = ref('')

const canSubmit = computed(
  () => !submitting.value && code.value.trim().length >= 2 && name.value.trim().length > 0,
)

watch(
  () => props.open,
  (open) => {
    if (!open) return
    code.value = ''
    name.value = ''
    description.value = ''
    permissionCodes.value = []
    submitting.value = false
    error.value = ''
  },
)

async function submit() {
  if (!canSubmit.value) return
  error.value = ''
  submitting.value = true
  try {
    const role = await api.createRole({
      code: code.value.trim(),
      name: name.value.trim(),
      description: description.value.trim(),
      permission_codes: [...permissionCodes.value],
    })
    emit('saved', role)
  } catch (caught) {
    error.value = errorMessage(caught, '创建角色失败')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="open"
    title="新建角色"
    description="角色编码用于后端鉴权与审计，创建后不可修改；权限可随时调整。"
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

    <div class="space-y-4">
      <div class="grid grid-cols-2 gap-4">
        <FormField
          label="角色编码"
          input-id="role-code"
          required
          hint="2-32 个字符，字母开头，可含数字、下划线、短横线"
        >
          <input
            id="role-code"
            v-model="code"
            type="text"
            autocomplete="off"
            spellcheck="false"
            placeholder="例如 operator"
            class="input"
          />
        </FormField>
        <FormField label="角色名称" input-id="role-name" required hint="展示在用户与角色列表中">
          <input
            id="role-name"
            v-model="name"
            type="text"
            autocomplete="off"
            placeholder="例如 运维值班"
            class="input"
          />
        </FormField>
      </div>

      <FormField label="描述" input-id="role-description" hint="可选，说明该角色的职责范围">
        <textarea
          id="role-description"
          v-model="description"
          rows="2"
          placeholder="例如：可维护房间与设备，但不能管理用户"
          class="textarea min-h-[62px]"
        />
      </FormField>

      <div>
        <p class="mb-1.5 text-[11.5px] font-medium text-ink-soft">权限点</p>
        <PermissionPicker v-model="permissionCodes" :permissions="permissions" />
      </div>
    </div>

    <template #footer>
      <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
      <AppButton variant="primary" :loading="submitting" :disabled="!canSubmit" @click="submit">
        创建角色
      </AppButton>
    </template>
  </AppModal>
</template>
