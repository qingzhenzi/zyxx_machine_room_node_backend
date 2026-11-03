<script setup lang="ts">
/**
 * 工单列表筛选栏：优先级 / 分类 / 处理人（含未分配）/ 只看我的 / 时间范围 / 重置。
 * 纯展示组件，通过 v-model:xxx 把状态交给 TicketsView，便于 URL query 同步。
 */
import AppButton from '@/components/ui/AppButton.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
import {
  TICKET_CATEGORY_ORDER,
  TICKET_PRIORITY_ORDER,
  ticketCategoryMeta,
  ticketPriorityMeta,
} from './ticketMeta'

interface AssigneeOption {
  id?: string | null
  display_name?: string | null
  username?: string | null
}

const props = withDefaults(
  defineProps<{
    assignees?: AssigneeOption[]
    canFilterMine?: boolean
  }>(),
  { assignees: () => [], canFilterMine: true },
)

const priority = defineModel<string>('priority', { required: true })
const category = defineModel<string>('category', { required: true })
const assignee = defineModel<string>('assignee', { required: true })
const mine = defineModel<boolean>('mine', { required: true })
const range = defineModel<string>('range', { required: true })

const emit = defineEmits<{
  reset: []
}>()

const TIME_RANGES = [
  { value: 'all', label: '全部时间' },
  { value: '7', label: '近 7 天' },
  { value: '30', label: '近 30 天' },
]

function assigneeLabel(person: AssigneeOption): string {
  return person.display_name || person.username || person.id || '未知用户'
}
</script>

<template>
  <div class="flex flex-wrap items-center gap-2">
    <select v-model="priority" class="select w-[132px]" aria-label="按优先级筛选工单">
      <option value="">全部优先级</option>
      <option v-for="key in TICKET_PRIORITY_ORDER" :key="key" :value="key">
        {{ ticketPriorityMeta(key).label }}
      </option>
    </select>

    <select v-model="category" class="select w-[140px]" aria-label="按分类筛选工单">
      <option value="">全部分类</option>
      <option v-for="key in TICKET_CATEGORY_ORDER" :key="key" :value="key">
        {{ ticketCategoryMeta(key).label }}
      </option>
    </select>

    <select v-model="assignee" class="select w-[150px]" aria-label="按处理人筛选工单">
      <option value="">全部处理人</option>
      <option value="unassigned">未分配</option>
      <option
        v-for="person in props.assignees"
        :key="person.id ?? assigneeLabel(person)"
        :value="person.id ?? ''"
      >
        {{ assigneeLabel(person) }}
      </option>
    </select>

    <label v-if="props.canFilterMine" class="flex items-center gap-2 text-[11.5px] text-muted">
      <ToggleSwitch v-model="mine" />
      只看我的
    </label>

    <select v-model="range" class="select w-[130px]" aria-label="按创建时间筛选工单">
      <option v-for="item in TIME_RANGES" :key="item.value" :value="item.value">
        {{ item.label }}
      </option>
    </select>

    <AppButton size="sm" variant="ghost" @click="emit('reset')">重置筛选</AppButton>
  </div>
</template>
