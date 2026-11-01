<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink } from 'vue-router'

import AppIcon from '@/components/ui/AppIcon.vue'
import NotificationsMenu from '@/components/layout/NotificationsMenu.vue'
import { useAuthStore } from '@/stores/auth'
import { useUiStore } from '@/stores/ui'
import { initials } from '@/utils/format'

defineProps<{
  pageTitle: string
}>()

const ui = useUiStore()
const auth = useAuthStore()
const userInitials = computed(() => initials(auth.displayName))
</script>

<template>
  <header class="flex h-14 shrink-0 items-center gap-5 border-b border-line bg-surface px-6">
    <div class="flex shrink-0 items-center gap-2 text-[12px]">
      <span class="text-muted">控制台</span>
      <span class="text-faint">/</span>
      <span class="font-medium text-ink">{{ pageTitle }}</span>
    </div>

    <button
      type="button"
      aria-label="全局搜索"
      class="relative mx-auto flex h-9 w-full max-w-[400px] items-center gap-2 rounded-md border border-line bg-canvas/60 px-3 text-left transition-colors hover:border-line-strong hover:bg-surface"
      @click="ui.openCommandPalette()"
    >
      <AppIcon name="search" :size="15" class="text-faint" />
      <span class="flex-1 truncate text-[12.5px] text-faint">搜索房间、设备、镜像或公告</span>
      <kbd
        class="shrink-0 rounded border border-line bg-surface px-1.5 py-0.5 font-mono text-[10px] leading-none text-faint"
      >
        ⌘K
      </kbd>
    </button>

    <div class="flex shrink-0 items-center gap-2">
      <a
        href="/docs"
        target="_blank"
        rel="noreferrer"
        class="inline-flex h-9 items-center gap-1.5 rounded-md px-2.5 text-[12px] text-muted transition-colors hover:bg-canvas hover:text-ink"
      >
        <AppIcon name="terminal" :size="14" />
        API 文档
      </a>

      <NotificationsMenu />

      <RouterLink
        to="/account"
        aria-label="账户设置"
        class="flex h-8 w-8 items-center justify-center rounded-full bg-accent-soft text-[11px] font-semibold text-accent ring-1 ring-accent-line/70 transition-colors hover:bg-accent-line/30"
      >
        {{ userInitials }}
      </RouterLink>
    </div>
  </header>
</template>
