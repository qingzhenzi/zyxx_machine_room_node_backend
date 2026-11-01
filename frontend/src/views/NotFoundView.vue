<script setup lang="ts">
import { useRouter } from 'vue-router'

import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'

const router = useRouter()
</script>

<template>
  <div class="mx-auto flex max-w-[560px] flex-col items-center px-6 py-24 text-center">
    <div
      class="flex h-12 w-12 items-center justify-center rounded-full border border-line bg-surface text-faint"
    >
      <AppIcon name="search" :size="20" />
    </div>
    <h1 class="mt-5 text-[20px] font-semibold tracking-[-0.02em] text-ink">页面不存在</h1>
    <p class="mt-2 text-[12px] leading-5 text-muted">链接可能已失效，或该资源已被删除。</p>
    <div class="mt-6 flex items-center gap-2">
      <AppButton @click="router.back()">返回上一页</AppButton>
      <AppButton variant="primary" @click="router.push({ name: 'overview' })">回到总览</AppButton>
    </div>
  </div>
</template>
