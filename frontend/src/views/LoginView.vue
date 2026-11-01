<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api } from '@/api'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { safeRedirect } from '@/utils/redirect'

const auth = useAuthStore()
const router = useRouter()
const route = useRoute()
const toast = useToastStore()

const apiBase = import.meta.env.VITE_API_BASE_URL || '/api/v1'

const username = ref('')
const password = ref('')
const remember = ref(true)
const showPassword = ref(false)
const capsLock = ref(false)
const submitting = ref(false)
const error = ref('')
const usernameInput = ref<HTMLInputElement | null>(null)

const forgotOpen = ref(false)
const forgotEmail = ref('')
const forgotSubmitting = ref(false)

const canSubmit = computed(
  () => username.value.trim().length > 0 && password.value.length > 0 && !submitting.value,
)

function onCapsLock(event: KeyboardEvent) {
  if (typeof event.getModifierState === 'function') {
    capsLock.value = event.getModifierState('CapsLock')
  }
}

async function submit() {
  if (!canSubmit.value) return

  error.value = ''
  submitting.value = true
  try {
    await auth.login(username.value.trim(), password.value, remember.value)
    await router.replace(safeRedirect(route.query.redirect))
  } catch (caught) {
    error.value = errorMessage(caught, '登录失败，请检查账号与密码')
    password.value = ''
  } finally {
    submitting.value = false
  }
}

async function requestReset() {
  if (!forgotEmail.value.trim()) return
  forgotSubmitting.value = true
  try {
    await api.forgotPassword(forgotEmail.value.trim())
    forgotOpen.value = false
    forgotEmail.value = ''
    toast.success('请求已受理', '如果邮箱存在，重置链接会写入后端日志或发送到邮箱')
  } catch (caught) {
    toast.error('请求失败', errorMessage(caught))
  } finally {
    forgotSubmitting.value = false
  }
}

onMounted(() => {
  void nextTick(() => usernameInput.value?.focus())
})
</script>

<template>
  <main class="flex min-h-dvh flex-col items-center justify-center bg-canvas px-6 py-16">
    <div class="mb-7 flex items-center gap-2">
      <span class="flex h-7 w-7 items-center justify-center rounded-md bg-ink text-white">
        <AppIcon name="door" :size="14" />
      </span>
      <span class="text-[14px] font-semibold tracking-[-0.01em] text-ink">Room Node Console</span>
      <span
        class="rounded border border-line px-1.5 py-px font-mono text-[9.5px] leading-4 text-faint"
      >
        v0.1
      </span>
    </div>

    <div
      class="w-full max-w-[360px] rounded-xl border border-line bg-surface px-8 py-8 shadow-[0_1px_2px_rgba(16,24,40,0.05)]"
    >
      <h1 class="text-[18px] font-semibold tracking-[-0.015em] text-ink">登录</h1>
      <p class="mt-1 text-[12px] text-muted">使用房间节点管理账号继续</p>

      <div
        v-if="error"
        role="alert"
        class="mt-4 flex items-start gap-2 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] leading-5 text-rose-700"
      >
        <AppIcon name="alert" :size="13" class="mt-0.5 shrink-0" />
        <span>{{ error }}</span>
      </div>

      <form class="mt-5 space-y-3.5" novalidate @submit.prevent="submit">
        <div>
          <label for="login-username" class="mb-1.5 block text-[11.5px] font-medium text-ink-soft">
            账号
          </label>
          <input
            id="login-username"
            ref="usernameInput"
            v-model="username"
            type="text"
            name="username"
            autocomplete="username"
            spellcheck="false"
            placeholder="用户名"
            class="h-10 w-full rounded-md border border-line bg-surface px-3 text-[13px] text-ink outline-none transition-colors placeholder:text-faint focus:border-accent focus:ring-2 focus:ring-accent/15"
          />
        </div>

        <div>
          <label for="login-password" class="mb-1.5 block text-[11.5px] font-medium text-ink-soft">
            密码
          </label>
          <div class="relative">
            <input
              id="login-password"
              v-model="password"
              :type="showPassword ? 'text' : 'password'"
              name="password"
              autocomplete="current-password"
              placeholder="密码"
              class="h-10 w-full rounded-md border border-line bg-surface px-3 pr-10 text-[13px] text-ink outline-none transition-colors placeholder:text-faint focus:border-accent focus:ring-2 focus:ring-accent/15"
              @keydown="onCapsLock"
              @keyup="onCapsLock"
            />
            <button
              type="button"
              :aria-label="showPassword ? '隐藏密码' : '显示密码'"
              class="absolute right-2 top-1/2 -translate-y-1/2 rounded p-1.5 text-faint transition-colors hover:text-ink"
              @click="showPassword = !showPassword"
            >
              <AppIcon :name="showPassword ? 'eye-off' : 'eye'" :size="14" />
            </button>
          </div>
          <p v-if="capsLock" class="mt-1.5 text-[10.5px] text-amber-600">大写锁定已开启</p>
        </div>

        <div class="flex items-center justify-between pt-0.5">
          <label class="flex cursor-pointer items-center gap-1.5 text-[11.5px] text-muted">
            <input
              v-model="remember"
              type="checkbox"
              class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
            />
            记住这台设备
          </label>
          <button
            type="button"
            class="text-[11.5px] text-accent transition-colors hover:text-accent-strong"
            @click="forgotOpen = true"
          >
            忘记密码？
          </button>
        </div>

        <AppButton
          type="submit"
          variant="primary"
          class="mt-1 w-full"
          :loading="submitting"
          :disabled="!canSubmit"
        >
          登录
        </AppButton>
      </form>
    </div>

    <p class="mt-6 text-center text-[10.5px] leading-5 text-faint">
      API 地址：{{ apiBase }} · 账号由管理员在「用户」页创建
    </p>

    <AppModal
      :open="forgotOpen"
      title="重置密码"
      description="输入账号绑定的邮箱。若邮箱存在，后端会写入重置链接（开发环境在日志中）。"
      size="sm"
      @close="forgotOpen = false"
    >
      <label for="forgot-email" class="field-label">邮箱</label>
      <input
        id="forgot-email"
        v-model="forgotEmail"
        type="email"
        autocomplete="email"
        placeholder="name@example.com"
        class="input"
        @keyup.enter="requestReset"
      />
      <template #footer>
        <AppButton :disabled="forgotSubmitting" @click="forgotOpen = false">取消</AppButton>
        <AppButton variant="primary" :loading="forgotSubmitting" @click="requestReset">
          发送重置请求
        </AppButton>
      </template>
    </AppModal>
  </main>
</template>
