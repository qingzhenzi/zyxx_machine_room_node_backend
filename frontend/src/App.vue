<script setup lang="ts">
import { computed } from 'vue'
import { RouterView, useRoute } from 'vue-router'

import PageProgress from '@/components/layout/PageProgress.vue'
import CommandPalette from '@/components/search/CommandPalette.vue'
import ToastHost from '@/components/ui/ToastHost.vue'

const route = useRoute()

// 登录页不挂工作台的全局面板（命令面板依赖登录后的接口）。
const workspaceShell = computed(() => !route.meta.public)
</script>

<template>
  <RouterView />
  <PageProgress />
  <CommandPalette v-if="workspaceShell" />
  <ToastHost />
</template>
