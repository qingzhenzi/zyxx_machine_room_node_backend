<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api } from '@/api'
import type { UserSession } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatDateTime, formatFullDateTime, formatRelative } from '@/utils/format'

const emit = defineEmits<{
  'signed-out': []
}>()

const toast = useToastStore()

const sessions = ref<UserSession[]>([])
const loading = ref(false)
const error = ref('')
const revokingId = ref<string | null>(null)
const revokingAll = ref(false)

const confirmOpen = ref(false)
const confirmMode = ref<'single' | 'all'>('single')
const confirmTarget = ref<UserSession | null>(null)

const activeSessions = computed(() => sessions.value.filter((session) => !session.revoked_at))

async function load() {
  loading.value = true
  error.value = ''
  try {
    sessions.value = await api.listSessions()
  } catch (caught) {
    error.value = errorMessage(caught, '加载设备会话失败')
    sessions.value = []
  } finally {
    loading.value = false
  }
}

function openRevoke(session: UserSession) {
  confirmTarget.value = session
  confirmMode.value = 'single'
  confirmOpen.value = true
}

function openRevokeAll() {
  confirmTarget.value = null
  confirmMode.value = 'all'
  confirmOpen.value = true
}

async function confirmRevoke() {
  if (confirmMode.value === 'all') {
    revokingAll.value = true
    try {
      await api.revokeAllSessions()
      confirmOpen.value = false
      toast.success('已撤销全部设备会话', '当前设备也需要重新登录')
      emit('signed-out')
    } catch (caught) {
      toast.error('撤销全部会话失败', errorMessage(caught, '请稍后重试'))
    } finally {
      revokingAll.value = false
    }
    return
  }

  const id = confirmTarget.value?.id
  if (!id) return
  revokingId.value = id
  try {
    await api.revokeSession(id)
    confirmOpen.value = false
    toast.success('设备会话已撤销')
    confirmTarget.value = null
    void load()
  } catch (caught) {
    toast.error('撤销会话失败', errorMessage(caught, '会话可能已被撤销'))
    void load()
  } finally {
    revokingId.value = null
  }
}

onMounted(() => void load())
</script>

<template>
  <section class="rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">设备会话</h2>
        <p class="mt-0.5 text-[11px] text-muted">
          {{ activeSessions.length }} 个有效会话 · 共 {{ sessions.length }} 条记录
        </p>
      </div>
      <div class="flex items-center gap-2">
        <IconButton
          icon="refresh"
          label="刷新设备会话"
          variant="outline"
          :loading="loading"
          @click="load()"
        />
        <AppButton
          size="sm"
          variant="danger"
          icon="logout"
          :disabled="activeSessions.length === 0"
          @click="openRevokeAll"
        >
          撤销全部
        </AppButton>
      </div>
    </header>

    <div
      v-if="error"
      class="mx-5 mt-3 flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <div v-if="loading" class="divide-y divide-line/70">
      <div v-for="row in 3" :key="`skeleton-${row}`" class="px-5 py-3">
        <span class="block h-3 w-2/3 animate-pulse rounded bg-canvas" />
        <span class="mt-2 block h-2.5 w-1/3 animate-pulse rounded bg-canvas" />
      </div>
    </div>

    <ul v-else-if="sessions.length" class="divide-y divide-line/70">
      <li
        v-for="session in sessions"
        :key="session.id"
        class="flex items-start justify-between gap-3 px-5 py-3"
      >
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <p class="truncate font-mono text-[11px] text-ink-soft" :title="session.user_agent">
              {{ session.user_agent || '未知客户端' }}
            </p>
            <StateBadge v-if="session.revoked_at" label="已撤销" tone="slate" :dot="false" />
            <StateBadge v-else label="有效" tone="emerald" :dot="false" />
          </div>
          <p class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[10.5px] text-faint">
            <span>IP：{{ session.ip || '—' }}</span>
            <span :title="formatFullDateTime(session.created_at)">
              创建：{{ formatDateTime(session.created_at) }}
            </span>
            <span :title="formatFullDateTime(session.last_seen_at)">
              最近活动：{{ formatRelative(session.last_seen_at) }}
            </span>
            <span v-if="session.revoked_at"> 撤销于 {{ formatDateTime(session.revoked_at) }} </span>
          </p>
        </div>
        <AppButton
          size="sm"
          variant="danger"
          :disabled="!!session.revoked_at"
          :loading="!!session.id && revokingId === session.id"
          @click="openRevoke(session)"
        >
          撤销
        </AppButton>
      </li>
    </ul>

    <EmptyState
      v-else
      title="没有设备会话"
      description="登录成功后设备会话会出现在这里，可撤销不再使用的客户端。"
      icon="monitor"
    />

    <template v-if="sessions.length">
      <footer class="border-t border-line px-5 py-2.5 text-[10.5px] text-faint">
        撤销会话后该客户端的 refresh token 立即失效；撤销全部会包含当前设备，需要重新登录。
      </footer>
    </template>

    <ConfirmDialog
      :open="confirmOpen"
      :title="confirmMode === 'all' ? '撤销全部设备会话' : '撤销设备会话'"
      :description="
        confirmMode === 'all'
          ? '确认撤销当前账号的全部设备会话？'
          : `确认撤销该设备会话（${confirmTarget?.user_agent || '未知客户端'}）？`
      "
      :detail="
        confirmMode === 'all'
          ? '包括当前设备在内的所有 refresh token 会立即失效，你需要重新登录。'
          : '该客户端的 refresh token 将立即失效，需要重新登录。'
      "
      confirm-label="撤销会话"
      danger
      :loading="revokingAll || !!revokingId"
      @close="confirmOpen = false"
      @confirm="confirmRevoke"
    />
  </section>
</template>
