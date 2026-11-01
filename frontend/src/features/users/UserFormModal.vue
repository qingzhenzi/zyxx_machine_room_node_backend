<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { api } from '@/api'
import type { RoleSummary, UserSummary } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
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

const editing = computed(() => props.user !== null)
const isSelf = computed(() => !!props.user?.id && props.user.id === auth.user?.id)

const username = ref('')
const displayName = ref('')
const email = ref('')
const password = ref('')
const isActive = ref(true)
const roleCodes = ref<string[]>([])
const submitting = ref(false)
const error = ref('')

const canSubmit = computed(() => {
  if (submitting.value) return false
  if (displayName.value.trim().length === 0) return false
  if (!editing.value) {
    return username.value.trim().length >= 3 && password.value.length >= 8
  }
  return true
})

watch(
  () => props.open,
  (open) => {
    if (!open) return
    error.value = ''
    submitting.value = false
    username.value = props.user?.username ?? ''
    displayName.value = props.user?.display_name ?? ''
    email.value = props.user?.email ?? ''
    password.value = ''
    isActive.value = props.user?.is_active ?? true
    roleCodes.value = [...(props.user?.roles ?? [])]
  },
)

function toggleRole(code: string) {
  const index = roleCodes.value.indexOf(code)
  if (index >= 0) roleCodes.value.splice(index, 1)
  else roleCodes.value.push(code)
}

async function submit() {
  if (!canSubmit.value) return
  error.value = ''
  submitting.value = true
  try {
    let saved: UserSummary
    if (props.user?.id) {
      saved = await api.updateUser(props.user.id, {
        display_name: displayName.value.trim(),
        email: email.value.trim(),
        is_active: isActive.value,
        ...(password.value ? { password: password.value } : {}),
      })
    } else {
      saved = await api.createUser({
        username: username.value.trim(),
        display_name: displayName.value.trim(),
        email: email.value.trim() || undefined,
        password: password.value,
        role_codes: [...roleCodes.value],
      })
    }
    emit('saved', saved)
  } catch (caught) {
    error.value = errorMessage(caught, '保存用户失败')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="open"
    :title="editing ? '编辑用户' : '新建用户'"
    :description="
      editing
        ? '修改资料、启用状态或重置密码；角色在「设置角色」中单独调整。'
        : '创建管理后台账号，并分配初始角色。'
    "
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

    <form class="space-y-4" @submit.prevent="submit">
      <div v-if="!editing" class="grid grid-cols-2 gap-4">
        <FormField
          label="用户名"
          input-id="user-username"
          required
          hint="3-32 个字符，字母开头，可含数字、下划线、短横线、点"
        >
          <input
            id="user-username"
            v-model="username"
            type="text"
            autocomplete="off"
            spellcheck="false"
            placeholder="例如 dev-operator"
            class="input"
          />
        </FormField>
        <FormField label="显示名称" input-id="user-display-name" required hint="1-64 个字符">
          <input
            id="user-display-name"
            v-model="displayName"
            type="text"
            autocomplete="off"
            placeholder="例如 运维值班"
            class="input"
          />
        </FormField>
      </div>

      <FormField v-else label="显示名称" input-id="user-display-name" required hint="1-64 个字符">
        <input
          id="user-display-name"
          v-model="displayName"
          type="text"
          autocomplete="off"
          class="input"
        />
      </FormField>

      <div class="grid grid-cols-2 gap-4">
        <FormField
          label="邮箱"
          input-id="user-email"
          :hint="editing ? '留空表示清空邮箱' : '可选；用于密码重置与邮箱验证'"
        >
          <input
            id="user-email"
            v-model="email"
            type="email"
            autocomplete="off"
            placeholder="name@example.com"
            class="input"
          />
        </FormField>

        <FormField
          label="密码"
          input-id="user-password"
          :required="!editing"
          :hint="editing ? '留空表示不修改，长度 8-128 个字符' : '长度 8-128 个字符'"
        >
          <input
            id="user-password"
            v-model="password"
            type="password"
            autocomplete="new-password"
            :placeholder="editing ? '留空不修改' : '至少 8 位'"
            class="input"
          />
        </FormField>
      </div>

      <div
        v-if="editing"
        class="flex items-center justify-between rounded-md border border-line px-3 py-2.5"
      >
        <div class="min-w-0">
          <p class="text-[11.5px] font-medium text-ink-soft">启用账号</p>
          <p class="mt-0.5 text-[10.5px] text-faint">
            <template v-if="isSelf">不能停用当前登录账号，后端会拒绝该操作。</template>
            <template v-else>停用后该账号无法登录，但仍保留历史审计记录。</template>
          </p>
        </div>
        <ToggleSwitch v-model="isActive" :disabled="isSelf" />
      </div>

      <div v-if="!editing">
        <p class="mb-1.5 text-[11.5px] font-medium text-ink-soft">初始角色</p>
        <div
          v-if="roles.length"
          class="grid max-h-[168px] grid-cols-2 gap-1 overflow-y-auto rounded-md border border-line p-2"
        >
          <label
            v-for="role in roles"
            :key="role.code"
            class="flex cursor-pointer items-start gap-2 rounded px-2 py-1.5 transition-colors hover:bg-canvas/70"
          >
            <input
              type="checkbox"
              :checked="roleCodes.includes(role.code ?? '')"
              class="mt-0.5 h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
              @change="toggleRole(role.code ?? '')"
            />
            <span class="min-w-0">
              <span class="block text-[11.5px] text-ink">{{ role.name || role.code }}</span>
              <span class="block truncate font-mono text-[10px] text-faint">{{ role.code }}</span>
            </span>
          </label>
        </div>
        <p v-else class="text-[11px] text-faint">暂无可分配角色，后端会默认分配 viewer。</p>
        <p class="mt-1.5 text-[10.5px] text-faint">
          不勾选任何角色时，后端默认分配 viewer（访客）。
        </p>
      </div>

      <p
        class="rounded-md border border-line bg-canvas/60 px-3 py-2 text-[10.5px] leading-5 text-muted"
      >
        安全约束：不能停用当前登录账号（400）；不能停用或移除最后一个管理员（409）。
        系统内置角色不可删除，其权限也不可修改。
      </p>
    </form>

    <template #footer>
      <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
      <AppButton variant="primary" :loading="submitting" :disabled="!canSubmit" @click="submit">
        {{ editing ? '保存修改' : '创建用户' }}
      </AppButton>
    </template>
  </AppModal>
</template>
