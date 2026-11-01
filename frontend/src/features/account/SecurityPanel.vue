<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { api } from '@/api'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import FormField from '@/components/ui/FormField.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'

const auth = useAuthStore()
const toast = useToastStore()

const resetEmail = ref(auth.user?.email ?? '')
const verificationLoading = ref(false)
const resetLoading = ref(false)

const boundEmail = computed(() => auth.user?.email ?? '')

watch(
  () => auth.user?.email,
  (email) => {
    if (!resetEmail.value) resetEmail.value = email ?? ''
  },
  { immediate: true },
)

async function requestVerification() {
  if (!boundEmail.value) {
    toast.error('未绑定邮箱', '请先在「用户」页面为账号设置邮箱')
    return
  }
  verificationLoading.value = true
  try {
    await api.requestEmailVerification()
    toast.success('验证邮件已请求', '开发环境未配置邮件服务时，验证链接会写入后端日志')
  } catch (caught) {
    toast.error('请求邮箱验证失败', errorMessage(caught, '请稍后重试'))
  } finally {
    verificationLoading.value = false
  }
}

async function requestReset() {
  const email = resetEmail.value.trim()
  if (!email) {
    toast.error('请输入邮箱', '密码重置链接会发送到该邮箱')
    return
  }
  resetLoading.value = true
  try {
    await api.forgotPassword(email)
    toast.success(
      '重置请求已受理',
      '若邮箱存在，重置链接会写入后端日志或发送到邮箱；不会透露账号是否存在',
    )
  } catch (caught) {
    toast.error('请求密码重置失败', errorMessage(caught, '请稍后重试'))
  } finally {
    resetLoading.value = false
  }
}
</script>

<template>
  <section class="rounded-xl border border-line bg-surface">
    <header class="border-b border-line px-5 py-4">
      <h2 class="text-[13.5px] font-semibold text-ink">邮箱与密码</h2>
      <p class="mt-0.5 text-[11px] text-muted">邮箱验证与密码重置链接在开发环境写入后端日志</p>
    </header>

    <div class="space-y-4 px-5 py-4">
      <div class="flex items-start justify-between gap-3 rounded-md border border-line px-3 py-2.5">
        <div class="min-w-0">
          <p class="text-[11.5px] font-medium text-ink-soft">邮箱验证</p>
          <p class="mt-0.5 text-[10.5px] leading-5 text-muted">
            <template v-if="boundEmail">
              已绑定 <span class="font-mono">{{ boundEmail }}</span
              >，可请求验证邮件。
            </template>
            <template v-else>
              当前账号未绑定邮箱，需由管理员在「用户」页面设置后再请求验证。
            </template>
          </p>
        </div>
        <AppButton
          size="sm"
          :icon="boundEmail ? 'check-circle' : 'info'"
          :disabled="!boundEmail"
          :loading="verificationLoading"
          @click="requestVerification"
        >
          请求验证邮件
        </AppButton>
      </div>

      <div class="rounded-md border border-line px-3 py-2.5">
        <p class="text-[11.5px] font-medium text-ink-soft">密码重置</p>
        <p class="mt-0.5 text-[10.5px] leading-5 text-muted">
          输入账号邮箱请求一次性重置链接。无论邮箱是否存在，后端都会返回成功，避免账号枚举。
        </p>
        <div class="mt-2.5 flex items-end gap-2">
          <div class="min-w-0 flex-1">
            <FormField label="邮箱" input-id="reset-email">
              <input
                id="reset-email"
                v-model="resetEmail"
                type="email"
                autocomplete="email"
                placeholder="name@example.com"
                class="input"
                @keyup.enter="requestReset"
              />
            </FormField>
          </div>
          <AppButton
            :icon="resetLoading ? '' : 'lock'"
            :loading="resetLoading"
            @click="requestReset"
          >
            发送重置链接
          </AppButton>
        </div>
      </div>

      <p
        class="flex items-start gap-1.5 rounded-md border border-line bg-canvas/60 px-3 py-2 text-[10.5px] leading-5 text-muted"
      >
        <AppIcon name="info" :size="12" class="mt-1 shrink-0" />
        开发环境：验证与重置链接打印在后端日志中；生产环境通过 SMTP（compose.mail.yaml）发送。
        重置密码成功后所有设备会话会被撤销。
      </p>
    </div>
  </section>
</template>
