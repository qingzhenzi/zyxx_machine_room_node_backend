<script setup lang="ts">
import { ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type { EffectiveAgentPolicyReply, Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import { POLICY_FIELDS, formatPolicyValue } from '@/features/agentConfigs/policy'
import { errorMessage } from '@/utils/errors'

const props = defineProps<{
  rooms: Room[]
}>()

const roomId = ref('')
const result = ref<EffectiveAgentPolicyReply | null>(null)
const loading = ref(false)
const error = ref('')

async function load() {
  if (!roomId.value) {
    result.value = null
    return
  }
  loading.value = true
  error.value = ''
  try {
    result.value = await api.effectiveAgentConfig(roomId.value)
  } catch (caught) {
    result.value = null
    error.value = isForbidden(caught)
      ? '缺少 device:read 权限，无法读取生效策略。'
      : errorMessage(caught, '加载生效策略失败')
  } finally {
    loading.value = false
  }
}

watch(
  () => props.rooms,
  (rooms) => {
    if (!roomId.value && rooms.length > 0) roomId.value = rooms[0]?.id ?? ''
  },
  { immediate: true },
)

watch(roomId, () => void load())

function retry() {
  void load()
}
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">生效策略预览</h2>
        <p class="mt-0.5 text-[11px] text-muted">按 global → area → room 合并后的最终策略</p>
      </div>
      <span
        v-if="result"
        class="shrink-0 rounded border border-line bg-canvas/60 px-1.5 py-0.5 font-mono text-[10px] text-muted"
      >
        revision r{{ result.revision }}
      </span>
    </header>

    <div class="space-y-3 px-5 py-4">
      <label for="effective-room" class="field-label">选择房间</label>
      <select id="effective-room" v-model="roomId" class="select">
        <option value="" disabled>请选择房间</option>
        <option v-for="room in rooms" :key="room.id" :value="room.id">
          {{ room.name }}（{{ room.area_name }}）
        </option>
      </select>

      <div
        v-if="error"
        class="flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] text-rose-700"
      >
        <span>{{ error }}</span>
        <AppButton size="sm" variant="ghost" @click="retry">重试</AppButton>
      </div>

      <template v-if="loading">
        <div class="grid grid-cols-2 gap-2">
          <div
            v-for="row in 4"
            :key="`skeleton-${row}`"
            class="rounded-md border border-line px-3 py-2"
          >
            <span class="block h-2.5 w-16 animate-pulse rounded bg-canvas" />
            <span class="mt-2 block h-3.5 w-20 animate-pulse rounded bg-canvas" />
          </div>
        </div>
      </template>

      <template v-else-if="result">
        <div class="grid grid-cols-2 gap-2">
          <div
            v-for="field in POLICY_FIELDS"
            :key="field.key"
            class="rounded-md border border-line bg-canvas/40 px-3 py-2"
          >
            <p class="text-[10px] text-faint">{{ field.label }}</p>
            <p class="mt-0.5 text-[12px] font-medium text-ink">
              {{ formatPolicyValue(field.key, result.policy[field.key]) }}
            </p>
          </div>
        </div>
        <p class="flex items-start gap-1.5 text-[10.5px] leading-5 text-faint">
          <AppIcon name="info" :size="12" class="mt-1 shrink-0" />
          合并顺序 global → area → room；revision 取已应用配置行的最大值，无配置时为 0。
        </p>
      </template>

      <EmptyState
        v-else-if="rooms.length === 0"
        title="暂无可预览的房间"
        description="请先在「房间」页面创建房间。"
        icon="door"
      />

      <EmptyState
        v-else
        title="选择一个房间"
        description="选中房间后展示该房间合并后的生效策略与 revision。"
        icon="sliders"
      />
    </div>
  </section>
</template>
