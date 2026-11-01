<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'

import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import ProfileCard from '@/features/account/ProfileCard.vue'
import SecurityPanel from '@/features/account/SecurityPanel.vue'
import SessionsPanel from '@/features/account/SessionsPanel.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'

const auth = useAuthStore()
const router = useRouter()
const toast = useToastStore()

const logoutOpen = ref(false)
const loggingOut = ref(false)

async function logout() {
  loggingOut.value = true
  try {
    await auth.logout()
    toast.success('已退出登录')
    await router.replace({ name: 'login' })
  } finally {
    loggingOut.value = false
    logoutOpen.value = false
  }
}

async function onSignedOut() {
  auth.clear()
  await router.replace({ name: 'login' })
}
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="账户" description="个人资料、角色权限、设备会话、邮箱验证与密码重置。">
      <template #actions>
        <AppButton icon="logout" variant="danger" @click="logoutOpen = true">退出登录</AppButton>
      </template>
    </PageHeader>

    <div class="mt-4 grid grid-cols-12 items-start gap-4">
      <div class="col-span-5 flex flex-col gap-4">
        <ProfileCard />
        <SecurityPanel />
      </div>
      <div class="col-span-7">
        <SessionsPanel @signed-out="onSignedOut" />
      </div>
    </div>

    <ConfirmDialog
      :open="logoutOpen"
      title="退出登录"
      description="确认退出当前账号？"
      detail="退出后当前设备的 refresh token 会被注销，需要重新输入账号密码登录。"
      confirm-label="退出登录"
      :loading="loggingOut"
      @close="logoutOpen = false"
      @confirm="logout"
    />
  </div>
</template>
