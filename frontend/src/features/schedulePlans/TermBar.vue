<script setup lang="ts">
import { computed } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import StateBadge from '@/components/ui/StateBadge.vue'

import type { Term } from './types'

const props = withDefaults(
  defineProps<{
    terms: Term[]
    selectedTermId: string
    loading?: boolean
    error?: string
    canManage?: boolean
    dirty?: boolean
    planCount?: number
  }>(),
  { loading: false, error: '', canManage: false, dirty: false, planCount: 0 },
)

const emit = defineEmits<{
  'update:selectedTermId': [value: string]
  create: []
  edit: []
  delete: []
  retry: []
}>()

const selectedTerm = computed(
  () => props.terms.find((term) => term.id === props.selectedTermId) ?? null,
)

const optionLabel = (term: Term) => {
  const active = term.is_active ? '' : '（已停用）'
  return `${term.name}${active} · ${term.start_date} ~ ${term.end_date}`
}

function onSelect(event: Event) {
  const value = (event.target as HTMLSelectElement).value
  if (value === props.selectedTermId) return
  emit('update:selectedTermId', value)
}
</script>

<template>
  <section
    class="mt-3 flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 rounded-xl border border-line bg-surface px-4 py-2.5"
  >
    <div class="flex items-center gap-1.5">
      <AppIcon name="calendar" :size="14" class="text-faint" />
      <span class="text-[11px] text-faint">学期</span>
    </div>

    <div v-if="loading && !terms.length" class="h-8 w-[300px] animate-pulse rounded bg-line/60" />

    <template v-else-if="terms.length">
      <select
        :value="selectedTermId"
        class="select h-8 w-[320px] px-2.5 pr-7 text-[12px]"
        :disabled="dirty || loading"
        :title="dirty ? '当前计划有未保存修改，请先保存草稿或放弃修改' : '切换学期'"
        aria-label="选择学期"
        @change="onSelect"
      >
        <option v-for="term in terms" :key="term.id" :value="term.id">
          {{ optionLabel(term) }}
        </option>
      </select>
      <StateBadge
        v-if="selectedTerm"
        :label="selectedTerm.is_active ? '启用' : '已停用'"
        :tone="selectedTerm.is_active ? 'emerald' : 'slate'"
      />
      <span v-if="selectedTerm" class="font-mono text-[10.5px] text-faint">
        {{ selectedTerm.code }} · {{ planCount }} 个计划
      </span>
      <span v-if="dirty" class="flex items-center gap-1 text-[10.5px] text-amber-600">
        <AppIcon name="alert" :size="11" />
        有未保存修改，暂不能切换学期
      </span>
    </template>

    <p v-else class="text-[11.5px] text-muted">还没有学期，先创建一个学期作为排课计划的归属。</p>

    <div class="ml-auto flex shrink-0 items-center gap-1.5">
      <AppButton v-if="canManage" size="sm" icon="plus" :disabled="loading" @click="emit('create')">
        新建学期
      </AppButton>
      <AppButton
        v-if="canManage"
        size="sm"
        icon="pencil"
        :disabled="!selectedTerm || loading"
        @click="emit('edit')"
      >
        编辑
      </AppButton>
      <AppButton
        v-if="canManage"
        size="sm"
        variant="danger"
        icon="trash"
        :disabled="!selectedTerm || loading"
        @click="emit('delete')"
      >
        删除
      </AppButton>
      <AppButton v-if="error" size="sm" variant="ghost" icon="refresh" @click="emit('retry')">
        重试
      </AppButton>
    </div>
  </section>
</template>
