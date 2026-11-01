<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import type { EventRule, NewEventRule, Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
import { useToastStore } from '@/stores/toast'
import { TRIGGER_KINDS, triggerKindDescription } from '@/features/events/kinds'
import type { TriggerKind } from '@/features/events/kinds'

interface RuleDraft {
  trigger_kind: TriggerKind
  offset_minutes: number
  at_time: string
  enabled: boolean
}

const props = withDefaults(
  defineProps<{
    rooms: Room[]
    roomId: string
    rules: EventRule[]
    loading?: boolean
    error?: string
    saving?: boolean
    canManage?: boolean
  }>(),
  { loading: false, error: '', saving: false, canManage: false },
)

const emit = defineEmits<{
  'update:roomId': [value: string]
  reload: []
  save: [rules: NewEventRule[]]
}>()

const toast = useToastStore()
const rows = ref<RuleDraft[]>([])
const validationError = ref('')

const sortedRooms = computed(() =>
  [...props.rooms].sort((left, right) => left.name.localeCompare(right.name, 'zh-CN')),
)
const currentRoom = computed(() => props.rooms.find((room) => room.id === props.roomId) ?? null)
const description = computed(() =>
  currentRoom.value
    ? `${currentRoom.value.name} · ${currentRoom.value.timezone} · 共 ${rows.value.length} 条规则`
    : '选择房间后维护该房间的事件触发规则。',
)

watch(
  () => props.rules,
  (rules) => {
    rows.value = rules.map((rule) => ({
      trigger_kind: rule.trigger_kind ?? 'occurrence_start',
      offset_minutes: rule.offset_minutes ?? 0,
      at_time: rule.at_time ?? '',
      enabled: rule.enabled ?? true,
    }))
    validationError.value = ''
  },
  { immediate: true },
)

function addRule() {
  rows.value.push({
    trigger_kind: 'occurrence_start',
    offset_minutes: 0,
    at_time: '',
    enabled: true,
  })
  validationError.value = ''
}

function removeRule(index: number) {
  rows.value.splice(index, 1)
  validationError.value = ''
}

function reset() {
  rows.value = props.rules.map((rule) => ({
    trigger_kind: rule.trigger_kind ?? 'occurrence_start',
    offset_minutes: rule.offset_minutes ?? 0,
    at_time: rule.at_time ?? '',
    enabled: rule.enabled ?? true,
  }))
  validationError.value = ''
}

function validate(): string | null {
  if (!props.roomId) return '请先选择房间'
  if (rows.value.length > 50) return '一次最多设置 50 条触发规则'
  const seen = new Set<string>()
  for (const [index, row] of rows.value.entries()) {
    const position = `第 ${index + 1} 条规则`
    const offset = Number(row.offset_minutes)
    if (!Number.isInteger(offset) || offset < 0 || offset > 1440) {
      return `${position}的偏移需为 0-1440 之间的整数`
    }
    if (row.trigger_kind === 'daily_time') {
      if (!row.at_time) return `${position}（每日定时）需要设置触发时间`
    } else if (row.at_time) {
      row.at_time = ''
    }
    if (row.trigger_kind === 'occurrence_start_before' && offset <= 0) {
      return `${position}（课前提醒）的偏移必须大于 0 分钟`
    }
    const key = `${row.trigger_kind}|${offset}|${row.trigger_kind === 'daily_time' ? row.at_time : ''}`
    if (seen.has(key)) return `${position}与前面的规则完全重复`
    seen.add(key)
  }
  return null
}

function save() {
  const message = validate()
  if (message) {
    validationError.value = message
    toast.error('触发规则校验未通过', message)
    return
  }
  validationError.value = ''
  emit(
    'save',
    rows.value.map((row) => ({
      trigger_kind: row.trigger_kind,
      offset_minutes: Number(row.offset_minutes),
      at_time: row.trigger_kind === 'daily_time' ? row.at_time : null,
      enabled: row.enabled,
      config: {},
    })),
  )
}
</script>

<template>
  <PageSection>
    <template #header>
      <div class="flex items-start justify-between gap-4">
        <div>
          <h2 class="text-[13.5px] font-semibold text-ink">房间触发规则</h2>
          <p class="mt-0.5 text-[11px] text-muted">{{ description }}</p>
        </div>
        <div class="flex shrink-0 items-center gap-2">
          <IconButton
            icon="refresh"
            label="刷新触发规则"
            :loading="loading"
            :disabled="loading || saving || !roomId"
            @click="emit('reload')"
          />
          <AppButton
            v-if="canManage"
            size="sm"
            icon="rotate-ccw"
            :disabled="loading || saving || !roomId"
            @click="reset"
          >
            还原
          </AppButton>
          <AppButton
            v-if="canManage"
            size="sm"
            icon="plus"
            :disabled="loading || saving || !roomId"
            @click="addRule"
          >
            添加规则
          </AppButton>
          <AppButton
            v-if="canManage"
            size="sm"
            variant="primary"
            icon="check"
            :loading="saving"
            :disabled="loading || !roomId"
            @click="save"
          >
            保存规则
          </AppButton>
        </div>
      </div>
    </template>

    <div class="flex items-center justify-between gap-3 border-b border-line/70 px-5 py-3">
      <label class="flex items-center gap-2 text-[11.5px] text-muted">
        房间
        <select
          :value="roomId"
          class="select w-[240px]"
          aria-label="选择房间查看触发规则"
          @change="emit('update:roomId', ($event.target as HTMLSelectElement).value)"
        >
          <option value="" disabled>请选择房间</option>
          <option v-for="room in sortedRooms" :key="room.id" :value="room.id">
            {{ room.name }}{{ room.is_active ? '' : '（已停用）' }}
          </option>
        </select>
      </label>
      <p class="text-[10.5px] text-faint">
        保存为全量替换；偏移 0-1440 分钟，每日定时按房间时区解释。
      </p>
    </div>

    <div
      v-if="error"
      class="mx-5 mt-4 flex items-center justify-between gap-3 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" :disabled="!roomId" @click="emit('reload')"
        >重试</AppButton
      >
    </div>

    <div
      v-if="validationError"
      class="mx-5 mt-4 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
    >
      {{ validationError }}
    </div>

    <div v-if="loading" class="px-5 py-3">
      <div v-for="index in 3" :key="index" class="mb-2 h-8 animate-pulse rounded bg-line/60" />
    </div>

    <EmptyState
      v-else-if="!roomId"
      title="请选择房间"
      description="触发规则属于房间维度，选择房间后可查看并编辑它的全部规则。"
      icon="door"
    />

    <EmptyState
      v-else-if="!rows.length"
      title="该房间还没有触发规则"
      description="添加课程开始、课前提醒或每日定时等规则后，命中排课实例时会生成触发事件。"
      icon="zap"
    >
      <template v-if="canManage" #action>
        <AppButton size="sm" icon="plus" @click="addRule">添加第一条规则</AppButton>
      </template>
    </EmptyState>

    <div v-else class="overflow-x-auto">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">触发类型</th>
            <th class="table-th w-[130px]">偏移（分钟）</th>
            <th class="table-th w-[140px]">触发时间</th>
            <th class="table-th w-[100px]">启用</th>
            <th class="table-th pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="(row, index) in rows"
            :key="index"
            class="border-b border-line/70 last:border-b-0"
          >
            <td class="table-td pl-5">
              <select
                v-model="row.trigger_kind"
                class="select w-[300px]"
                :disabled="!canManage"
                :aria-label="`第 ${index + 1} 条规则触发类型`"
              >
                <option v-for="kind in TRIGGER_KINDS" :key="kind.value" :value="kind.value">
                  {{ kind.label }}
                </option>
              </select>
              <p class="mt-1 text-[10px] text-faint">
                {{ triggerKindDescription(row.trigger_kind) }}
              </p>
            </td>
            <td class="table-td">
              <input
                v-model.number="row.offset_minutes"
                type="number"
                min="0"
                max="1440"
                step="1"
                class="input input-sm w-[100px]"
                :disabled="!canManage"
                :aria-label="`第 ${index + 1} 条规则偏移分钟`"
              />
            </td>
            <td class="table-td">
              <input
                v-model="row.at_time"
                type="time"
                class="input input-sm w-[120px]"
                :disabled="!canManage || row.trigger_kind !== 'daily_time'"
                :placeholder="row.trigger_kind === 'daily_time' ? 'HH:MM' : '仅每日定时'"
                :aria-label="`第 ${index + 1} 条规则触发时间`"
              />
            </td>
            <td class="table-td">
              <ToggleSwitch v-model="row.enabled" :disabled="!canManage" />
            </td>
            <td class="table-td pr-5 text-right">
              <IconButton
                v-if="canManage"
                icon="trash"
                label="删除触发规则"
                size="sm"
                :disabled="saving"
                @click="removeRule(index)"
              />
              <span v-else class="text-[10.5px] text-faint">只读</span>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <p
      v-if="rows.length"
      class="flex items-center gap-1.5 border-t border-line/70 px-5 py-2.5 text-[10.5px] text-faint"
    >
      当前共 {{ rows.length }} 条规则；仅「每日定时」需要触发时间，其余类型使用偏移分钟。
    </p>
  </PageSection>
</template>
