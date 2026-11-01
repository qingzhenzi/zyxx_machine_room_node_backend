<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { RouterView, useRoute } from 'vue-router'

import AppSidebar from '@/components/layout/AppSidebar.vue'
import AppStatusBar from '@/components/layout/AppStatusBar.vue'
import AppTopbar from '@/components/layout/AppTopbar.vue'
import { useInterval } from '@/composables/useInterval'
import { useShellStore } from '@/stores/shell'
import { useUiStore } from '@/stores/ui'

const route = useRoute()
const ui = useUiStore()
const shell = useShellStore()

const pageTitle = computed(() => (route.meta.title as string | undefined) ?? '总览')
const mainRef = ref<HTMLElement | null>(null)

watch(
  () => route.fullPath,
  () => mainRef.value?.scrollTo({ top: 0 }),
)

function onGlobalKeydown(event: KeyboardEvent) {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
    event.preventDefault()
    ui.commandPaletteOpen = !ui.commandPaletteOpen
  }
}

onMounted(() => {
  window.addEventListener('keydown', onGlobalKeydown)
  void shell.bootstrap()
})

onUnmounted(() => {
  window.removeEventListener('keydown', onGlobalKeydown)
})

// 侧边栏摘要与通知中心每 30 秒刷新一次，保持与后台状态近似实时。
useInterval(() => {
  void shell.bootstrap(true)
}, 30_000)
</script>

<template>
  <div
    class="grid h-dvh min-h-[720px] min-w-[1280px] grid-cols-[248px_minmax(0,1fr)] overflow-hidden bg-canvas"
  >
    <AppSidebar />

    <div class="flex min-h-0 min-w-0 flex-col">
      <AppTopbar :page-title="pageTitle" />

      <main ref="mainRef" class="min-h-0 flex-1 overflow-y-auto">
        <RouterView />
      </main>

      <AppStatusBar />
    </div>
  </div>
</template>
