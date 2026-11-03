<script setup lang="ts">
import { computed } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import FilterTabs from '@/components/ui/FilterTabs.vue'
import IconButton from '@/components/ui/IconButton.vue'
import StateBadge from '@/components/ui/StateBadge.vue'

import type { Course, MasterDataType, MasterRecord, SchoolClass, Teacher } from './types'
import { MASTER_TYPES, masterTypeLabel } from './types'

const props = withDefaults(
  defineProps<{
    open: boolean
    type: MasterDataType
    items: MasterRecord[]
    counts: Record<MasterDataType, number>
    loading?: boolean
    error?: string
    canManage?: boolean
  }>(),
  { loading: false, error: '', canManage: false },
)

const emit = defineEmits<{
  'update:open': [value: boolean]
  'update:type': [value: MasterDataType]
  create: []
  edit: [record: MasterRecord]
  delete: [record: MasterRecord]
  retry: []
}>()

const tabs = computed(() =>
  MASTER_TYPES.map((item) => ({
    value: item.value,
    label: item.label,
    count: props.counts[item.value] ?? 0,
  })),
)

const typeLabel = computed(() => masterTypeLabel(props.type))
const extraHeader = computed(() => {
  if (props.type === 'course') return '颜色'
  if (props.type === 'teacher') return '联系方式'
  return '年级'
})

function extraOf(record: MasterRecord): { text: string; color?: string } {
  if (props.type === 'course') {
    const course = record as Course
    return { text: course.color || '默认', color: course.color || undefined }
  }
  if (props.type === 'teacher') {
    const teacher = record as Teacher
    const text = [teacher.phone, teacher.email].filter(Boolean).join(' · ') || '—'
    return { text }
  }
  const schoolClass = record as SchoolClass
  return { text: schoolClass.grade || '—' }
}
</script>

<template>
  <section class="mt-3 shrink-0 overflow-hidden rounded-xl border border-line bg-surface">
    <header class="flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-2">
      <button
        type="button"
        class="flex items-center gap-1.5 text-[12.5px] font-semibold text-ink"
        :aria-expanded="open"
        @click="emit('update:open', !open)"
      >
        <AppIcon :name="open ? 'chevron-down' : 'chevron-right'" :size="13" class="text-faint" />
        主数据
      </button>
      <span class="text-[10.5px] text-faint">
        课程 {{ counts.course ?? 0 }} · 教师 {{ counts.teacher ?? 0 }} · 班级
        {{ counts.class ?? 0 }}
      </span>
      <div class="ml-auto flex flex-wrap items-center gap-2">
        <FilterTabs
          v-if="open"
          :model-value="type"
          :tabs="tabs"
          @update:model-value="emit('update:type', $event as MasterDataType)"
        />
        <AppButton v-if="open && canManage" size="sm" icon="plus" @click="emit('create')">
          新建{{ typeLabel }}
        </AppButton>
      </div>
    </header>

    <div v-if="open" class="border-t border-line">
      <div
        v-if="error"
        class="flex items-center justify-between border-b border-rose-200 bg-rose-50 px-4 py-2 text-[11.5px] text-rose-700"
      >
        <span>{{ error }}</span>
        <AppButton size="sm" variant="ghost" @click="emit('retry')">重试</AppButton>
      </div>

      <div class="max-h-[224px] overflow-y-auto">
        <table v-if="loading || items.length" class="w-full border-collapse text-left">
          <thead class="sticky top-0 z-10">
            <tr class="border-b border-line bg-canvas/90 backdrop-blur">
              <th class="table-th w-[150px] pl-4">编码</th>
              <th class="table-th">名称</th>
              <th class="table-th w-[210px]">{{ extraHeader }}</th>
              <th class="table-th w-[88px]">状态</th>
              <th class="table-th w-[76px]">引用</th>
              <th class="table-th w-[88px] pr-4 text-right">操作</th>
            </tr>
          </thead>
          <tbody>
            <template v-if="loading && !items.length">
              <tr
                v-for="index in 3"
                :key="`master-skeleton-${index}`"
                class="border-b border-line/70"
              >
                <td colspan="6" class="px-4 py-2">
                  <div class="h-5 animate-pulse rounded bg-line/60" />
                </td>
              </tr>
            </template>
            <tr
              v-for="record in items"
              v-else
              :key="record.id"
              class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            >
              <td class="table-td pl-4 font-mono text-[10.5px] text-muted">{{ record.code }}</td>
              <td class="table-td text-[12px] font-medium text-ink">{{ record.name }}</td>
              <td class="table-td">
                <span class="flex items-center gap-1.5">
                  <span
                    v-if="extraOf(record).color"
                    class="h-2.5 w-2.5 shrink-0 rounded-sm border border-line"
                    :style="{ background: extraOf(record).color }"
                  />
                  <span class="truncate text-[11px] text-muted">{{ extraOf(record).text }}</span>
                </span>
              </td>
              <td class="table-td">
                <StateBadge
                  :label="record.is_active ? '启用' : '停用'"
                  :tone="record.is_active ? 'emerald' : 'slate'"
                />
              </td>
              <td class="table-td font-mono text-[10.5px] text-faint">{{ record.usage ?? 0 }}</td>
              <td class="table-td pr-4 text-right">
                <div v-if="canManage" class="flex items-center justify-end gap-0.5">
                  <IconButton
                    icon="pencil"
                    :label="`编辑${typeLabel}`"
                    size="sm"
                    @click="emit('edit', record)"
                  />
                  <IconButton
                    icon="trash"
                    :label="`删除${typeLabel}`"
                    size="sm"
                    @click="emit('delete', record)"
                  />
                </div>
                <span v-else class="text-[10.5px] text-faint">只读</span>
              </td>
            </tr>
          </tbody>
        </table>

        <EmptyState
          v-else-if="!error"
          :title="`还没有${typeLabel}`"
          :description="`在计划条目中引用${typeLabel}前，需要先创建一条记录。`"
          icon="layers"
        >
          <template v-if="canManage" #action>
            <AppButton size="sm" icon="plus" variant="primary" @click="emit('create')">
              新建{{ typeLabel }}
            </AppButton>
          </template>
        </EmptyState>
      </div>
    </div>
  </section>
</template>
