<script setup lang="ts">
import { computed } from 'vue'

import type { AgentConfig, Area, Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import { POLICY_FIELDS, patchValue, scopeLabel } from '@/features/agentConfigs/policy'
import { formatDateTime, shortId } from '@/utils/format'

const props = defineProps<{
  configs: AgentConfig[]
  areas: Area[]
  rooms: Room[]
  loading: boolean
  canManage: boolean
}>()

const emit = defineEmits<{
  create: []
  edit: [config: AgentConfig]
}>()

const areaNames = computed(() => new Map(props.areas.map((area) => [area.id, area.name])))
const roomNames = computed(() => new Map(props.rooms.map((room) => [room.id, room.name])))

const sortedConfigs = computed(() => {
  const order: Record<string, number> = { global: 0, area: 1, room: 2 }
  return [...props.configs].sort((left, right) => {
    const diff = (order[left.scope_type] ?? 9) - (order[right.scope_type] ?? 9)
    if (diff !== 0) return diff
    return (right.updated_at ?? '').localeCompare(left.updated_at ?? '')
  })
})

function scopeName(config: AgentConfig): string {
  if (config.scope_type === 'global') return '所有区域与房间'
  if (!config.scope_id) return '未指定作用域'
  const map = config.scope_type === 'area' ? areaNames.value : roomNames.value
  return map.get(config.scope_id) ?? `未知作用域 ${shortId(config.scope_id)}`
}

function scopeTone(scope: string): string {
  if (scope === 'global') return 'bg-violet-50 text-violet-700'
  if (scope === 'area') return 'bg-sky-50 text-sky-700'
  return 'bg-emerald-50 text-emerald-700'
}
</script>

<template>
  <section class="overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <h2 class="text-[13.5px] font-semibold text-ink">作用域配置</h2>
        <p class="mt-0.5 text-[11px] text-muted">
          策略按 global → area → room 合并；未设置的字段继承上层，保存后会二次下发 config.update。
        </p>
      </div>
      <span class="shrink-0 text-[11px] text-muted">共 {{ configs.length }} 份配置</span>
    </header>

    <table class="w-full border-collapse text-left">
      <thead>
        <tr class="border-b border-line bg-canvas/50">
          <th class="table-th pl-5">作用域</th>
          <th class="table-th">范围</th>
          <th class="table-th">Revision</th>
          <th class="table-th">策略 patch</th>
          <th class="table-th">更新人</th>
          <th class="table-th">更新时间</th>
          <th v-if="canManage" class="table-th pr-5 text-right">操作</th>
        </tr>
      </thead>
      <tbody>
        <template v-if="loading">
          <tr
            v-for="row in 3"
            :key="`skeleton-${row}`"
            class="border-b border-line/70 last:border-b-0"
          >
            <td v-for="col in canManage ? 7 : 6" :key="col" class="table-td">
              <span class="block h-3 animate-pulse rounded bg-canvas" />
            </td>
          </tr>
        </template>

        <template v-else>
          <tr
            v-for="config in sortedConfigs"
            :key="config.id"
            class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
          >
            <td class="table-td pl-5">
              <span
                class="rounded px-1.5 py-0.5 text-[10px] font-medium"
                :class="scopeTone(config.scope_type)"
              >
                {{ scopeLabel(config) }}
              </span>
            </td>
            <td class="table-td">
              <p class="text-[11.5px] text-ink">{{ scopeName(config) }}</p>
              <p v-if="config.scope_id" class="font-mono text-[9.5px] text-faint">
                {{ shortId(config.scope_id, 8) }}
              </p>
            </td>
            <td class="table-td">
              <span class="font-mono text-[11px] text-ink-soft">r{{ config.revision }}</span>
            </td>
            <td class="table-td">
              <div class="flex max-w-[420px] flex-wrap gap-1">
                <span
                  v-for="field in POLICY_FIELDS"
                  :key="field.key"
                  class="rounded border px-1.5 py-0.5 text-[10px]"
                  :class="
                    patchValue(config.policy, field.key) === '继承'
                      ? 'border-line bg-canvas/60 text-faint'
                      : 'border-line bg-surface text-ink-soft'
                  "
                  :title="field.label"
                >
                  {{ field.label }}：{{ patchValue(config.policy, field.key) }}
                </span>
              </div>
            </td>
            <td class="table-td">
              <span class="font-mono text-[10.5px] text-muted" :title="config.updated_by ?? ''">
                {{ shortId(config.updated_by, 8) }}
              </span>
            </td>
            <td class="table-td whitespace-nowrap text-[11px] text-muted">
              {{ formatDateTime(config.updated_at) }}
            </td>
            <td v-if="canManage" class="table-td pr-5">
              <div class="flex justify-end">
                <AppButton size="sm" @click="emit('edit', config)">编辑配置</AppButton>
              </div>
            </td>
          </tr>
        </template>
      </tbody>
    </table>

    <EmptyState
      v-if="!loading && !configs.length"
      title="还没有作用域配置"
      description="默认策略已生效（LXC 不自动启动、轮询 2 秒、下载并发 4、缓存 10 GiB）。可按作用域覆盖。"
      icon="sliders"
    >
      <template v-if="canManage" #action>
        <AppButton icon="plus" variant="primary" @click="emit('create')">新建配置</AppButton>
      </template>
    </EmptyState>
  </section>
</template>
