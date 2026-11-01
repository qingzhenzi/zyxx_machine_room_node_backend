<script setup lang="ts">
import { computed, ref } from 'vue'

import AppIcon from '@/components/ui/AppIcon.vue'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { formatDateTime, initials } from '@/utils/format'

const auth = useAuthStore()
const toast = useToastStore()

const refreshing = ref(false)

const userInitials = computed(() => initials(auth.displayName))
const roles = computed(() => auth.roles)
const permissions = computed(() => auth.permissions)

async function refresh() {
  refreshing.value = true
  try {
    await auth.refreshProfile()
    toast.success('资料已刷新', `${auth.roles.length} 个角色 · ${auth.permissions.length} 个权限点`)
  } finally {
    refreshing.value = false
  }
}
</script>

<template>
  <section class="rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">当前账号</h2>
        <p class="mt-0.5 text-[11px] text-muted">角色与权限点可能在后台被修改，可随时刷新</p>
      </div>
      <IconButton
        icon="refresh"
        label="刷新当前账号资料"
        variant="outline"
        :loading="refreshing"
        @click="refresh"
      />
    </header>

    <div v-if="auth.user" class="px-5 py-4">
      <div class="flex items-center gap-3">
        <span
          class="flex h-11 w-11 items-center justify-center rounded-full bg-accent-soft text-[14px] font-semibold text-accent ring-1 ring-accent-line/70"
        >
          {{ userInitials }}
        </span>
        <div class="min-w-0">
          <div class="flex items-center gap-2">
            <p class="truncate text-[14px] font-semibold text-ink">{{ auth.user.display_name }}</p>
            <StateBadge
              :label="auth.user.is_active ? '启用' : '停用'"
              :tone="auth.user.is_active ? 'emerald' : 'rose'"
            />
          </div>
          <p class="mt-0.5 font-mono text-[11px] text-muted">
            {{ auth.user.username }}
            <span v-if="auth.user.email" class="font-sans"> · {{ auth.user.email }}</span>
            <span v-else class="font-sans"> · 未绑定邮箱</span>
          </p>
        </div>
      </div>

      <dl class="mt-4 grid grid-cols-2 gap-x-6 gap-y-2.5 text-[11.5px]">
        <div>
          <dt class="text-[10.5px] text-faint">用户 ID</dt>
          <dd class="mt-0.5">
            <CopyText :value="auth.user.id" :display="auth.user.id" label="用户 ID" truncate />
          </dd>
        </div>
        <div>
          <dt class="text-[10.5px] text-faint">创建时间</dt>
          <dd class="mt-0.5 text-ink-soft">{{ formatDateTime(auth.user.created_at) }}</dd>
        </div>
      </dl>

      <div class="mt-4">
        <p class="mb-1.5 text-[10.5px] text-faint">角色（{{ roles.length }}）</p>
        <div v-if="roles.length" class="flex flex-wrap gap-1">
          <span
            v-for="role in roles"
            :key="role"
            class="rounded border border-line bg-canvas/60 px-1.5 py-0.5 font-mono text-[10px] text-ink-soft"
          >
            {{ role }}
          </span>
        </div>
        <p v-else class="text-[11px] text-faint">未分配角色</p>
      </div>

      <div class="mt-4">
        <p class="mb-1.5 flex items-center gap-1.5 text-[10.5px] text-faint">
          <AppIcon name="shield" :size="11" />
          权限点（{{ permissions.length }}）
        </p>
        <div
          v-if="permissions.length"
          class="max-h-[188px] overflow-y-auto rounded-md border border-line bg-canvas/40 p-2"
        >
          <div class="flex flex-wrap gap-1">
            <span
              v-for="permission in permissions"
              :key="permission"
              class="rounded bg-surface px-1.5 py-0.5 font-mono text-[9.5px] text-muted ring-1 ring-line"
            >
              {{ permission }}
            </span>
          </div>
        </div>
        <p v-else class="text-[11px] text-faint">暂无权限点</p>
      </div>
    </div>

    <EmptyState
      v-else
      title="未登录"
      description="当前没有可展示的账号资料，请重新登录。"
      icon="user"
    />
  </section>
</template>
