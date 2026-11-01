<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { api } from '@/api'
import type { RoleSummary, UserSummary } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
import { useAuthStore } from '@/stores/auth'
import { errorMessage } from '@/utils/errors'

const props = defineProps<{
  open: boolean
  user: UserSummary | null
  roles: RoleSummary[]
}>()

const emit = defineEmits<{
  close: []
  saved: [user: UserSummary]
}>()

const auth = useAuthStore()

const selected = ref<string[]>([])
const submitting = ref(false)
const error = ref('')

const isSelf = computed(() => !!props.user?.id && props.user.id === auth.user?.id)
const targetName = computed(() => props.user?.display_name || props.user?.username || '该用户')

watch(
  () => props.open,
  (open) => {
    if (!open) return
    error.value = ''
    submitting.value = false
    selected.value = [...(props.user?.roles ?? [])]
  },
)

function toggle(code: string) {
  const index = selected.value.indexOf(code)
  if (index >= 0) selected.value.splice(index, 1)
  else selected.value.push(code)
}

async function submit() {
  if (!props.user?.id) return
  error.value = ''
  submitting.value = true
  try {
    emit('saved', await api.setUserRoles(props.user.id, [...selected.value]))
  } catch (caught) {
    error.value = errorMessage(caught, '设置用户角色失败')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="open"
    title="设置角色"
    :description="`为 ${targetName} 调整角色，保存后立即生效。`"
    size="md"
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

    <p
      v-if="isSelf"
      class="mb-3 rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-[10.5px] leading-5 text-amber-700"
    >
      正在修改当前登录账号的角色。移除自己的管理权限会立即生效，请确认后操作。
    </p>

    <div
      v-if="roles.length"
      class="max-h-[320px] space-y-1 overflow-y-auto rounded-md border border-line p-2"
    >
      <label
        v-for="role in roles"
        :key="role.code"
        class="flex cursor-pointer items-start gap-2.5 rounded px-2 py-2 transition-colors hover:bg-canvas/70"
      >
        <input
          type="checkbox"
          :checked="selected.includes(role.code ?? '')"
          class="mt-0.5 h-3.5 w-3.5 shrink-0 rounded border-line text-accent focus:ring-accent/25"
          @change="toggle(role.code ?? '')"
        />
        <span class="min-w-0 flex-1">
          <span class="flex items-center gap-1.5">
            <span class="text-[12px] text-ink">{{ role.name || role.code }}</span>
            <span class="rounded bg-canvas px-1 py-px font-mono text-[9.5px] text-faint">
              {{ role.code }}
            </span>
            <span
              v-if="role.is_system"
              class="rounded bg-slate-100 px-1 py-px text-[9.5px] text-slate-600"
            >
              系统
            </span>
          </span>
          <span v-if="role.description" class="mt-0.5 block text-[10.5px] leading-4 text-muted">
            {{ role.description }}
          </span>
        </span>
      </label>
    </div>
    <p v-else class="text-[11.5px] text-faint">暂无可分配的角色。</p>

    <p
      class="mt-3 rounded-md border border-line bg-canvas/60 px-3 py-2 text-[10.5px] leading-5 text-muted"
    >
      至少保留一个管理员角色。移除最后一个管理员时后端返回 409，页面会展示具体原因。
    </p>

    <template #footer>
      <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
      <AppButton variant="primary" :loading="submitting" @click="submit">保存角色</AppButton>
    </template>
  </AppModal>
</template>
