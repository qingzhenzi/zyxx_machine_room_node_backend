<script setup lang="ts">
import { useRouter } from 'vue-router'

import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import { useAuthStore } from '@/stores/auth'

const auth = useAuthStore()
const router = useRouter()
</script>

<template>
  <div class="mx-auto flex max-w-[560px] flex-col items-center px-6 py-24 text-center">
    <div
      class="flex h-12 w-12 items-center justify-center rounded-full border border-line bg-surface text-faint"
    >
      <AppIcon name="lock" :size="20" />
    </div>
    <h1 class="mt-5 text-[20px] font-semibold tracking-[-0.02em] text-ink">没有访问权限</h1>
    <p class="mt-2 text-[12px] leading-5 text-muted">
      当前账号缺少访问该页面所需的权限点。请联系管理员调整角色，或返回总览。
    </p>
    <p class="mt-3 font-mono text-[11px] text-faint">角色：{{ auth.roles.join(', ') || '—' }}</p>
    <div class="mt-6 flex items-center gap-2">
      <AppButton @click="router.back()">返回上一页</AppButton>
      <AppButton variant="primary" @click="router.push({ name: 'overview' })">回到总览</AppButton>
    </div>
  </div>
</template>
