<script setup lang="ts">
import { computed } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import StateBadge from '@/components/ui/StateBadge.vue'

import type { PlanEntryDraft, PlanView } from './types'
import { draftSummary, planStatusMeta, sortDrafts } from './types'

const props = withDefaults(
  defineProps<{
    plan?: PlanView | null
    entries: PlanEntryDraft[]
    termName?: string
    loading?: boolean
    error?: string
    canManage?: boolean
    dirty?: boolean
    saving?: boolean
    publishing?: boolean
    cloning?: boolean
  }>(),
  {
    plan: null,
    termName: '',
    loading: false,
    error: '',
    canManage: false,
    dirty: false,
    saving: false,
    publishing: false,
    cloning: false,
  },
)

const emit = defineEmits<{
  addEntry: []
  editEntry: [entry: PlanEntryDraft]
  deleteEntry: [entry: PlanEntryDraft]
  save: []
  publish: []
  clone: []
  retry: []
  discard: []
}>()

const sortedEntries = computed(() => sortDrafts(props.entries))
const summary = computed(() => draftSummary(props.entries))
const statusMeta = computed(() => planStatusMeta(props.plan?.status ?? 'draft'))
const isDraft = computed(() => props.plan?.status === 'draft')
const busy = computed(() => props.saving || props.publishing || props.cloning)

const summaryParts = computed(() => {
  const value = summary.value
  return [
    `${value.entries} 条`,
    `${value.rooms} 房间`,
    `${value.courses} 课程`,
    `${Math.round((value.minutes / 60) * 10) / 10} 课时`,
  ]
})
</script>

<template>
  <section class="flex min-h-0 flex-col overflow-hidden rounded-xl border border-line bg-surface">
    <header
      class="flex shrink-0 flex-wrap items-start justify-between gap-3 border-b border-line px-4 py-2.5"
    >
      <div class="min-w-0">
        <div class="flex flex-wrap items-center gap-2">
          <h2 class="max-w-[360px] truncate text-[13.5px] font-semibold text-ink">
            {{ plan?.name ?? '条目编辑器' }}
          </h2>
          <StateBadge v-if="plan" :label="statusMeta.label" :tone="statusMeta.tone" />
          <span
            v-if="plan"
            class="rounded border border-line px-1 font-mono text-[9.5px] text-faint"
            >v{{ plan.version }}</span
          >
          <span
            v-if="dirty && isDraft"
            class="flex items-center gap-1 rounded bg-amber-50 px-1.5 py-0.5 text-[10px] font-medium text-amber-700"
          >
            <AppIcon name="alert" :size="10" />
            未保存
          </span>
        </div>
        <p v-if="plan" class="mt-0.5 flex flex-wrap items-center gap-1.5 text-[10.5px] text-muted">
          <span>{{ termName }}</span>
          <span class="h-2.5 w-px bg-line" />
          <span v-for="(part, index) in summaryParts" :key="part">
            {{ part }}<span v-if="index < summaryParts.length - 1"> · </span>
          </span>
        </p>
      </div>

      <div v-if="plan" class="flex shrink-0 items-center gap-1.5">
        <template v-if="canManage">
          <template v-if="isDraft">
            <AppButton size="sm" icon="plus" :disabled="busy" @click="emit('addEntry')">
              添加条目
            </AppButton>
            <AppButton
              size="sm"
              icon="rotate-ccw"
              :disabled="!dirty || busy"
              title="放弃未保存修改"
              @click="emit('discard')"
            >
              放弃
            </AppButton>
            <AppButton
              size="sm"
              icon="check"
              :loading="saving"
              :disabled="!dirty || busy"
              @click="emit('save')"
            >
              保存草稿
            </AppButton>
            <AppButton
              size="sm"
              variant="primary"
              icon="play-circle"
              :loading="publishing"
              :disabled="busy || !entries.length"
              @click="emit('publish')"
            >
              发布
            </AppButton>
          </template>
          <AppButton
            v-else
            size="sm"
            icon="copy"
            :loading="cloning"
            :disabled="busy"
            @click="emit('clone')"
          >
            克隆为新版本
          </AppButton>
        </template>
        <IconButton
          icon="refresh"
          label="刷新计划条目"
          variant="outline"
          :loading="loading"
          :disabled="loading"
          @click="emit('retry')"
        />
      </div>
    </header>

    <div
      v-if="error"
      class="flex shrink-0 items-center justify-between border-b border-rose-200 bg-rose-50 px-4 py-2 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="emit('retry')">重试</AppButton>
    </div>

    <div class="min-h-0 flex-1 overflow-auto">
      <div v-if="!plan && !loading" class="flex h-full items-center justify-center">
        <EmptyState
          title="选择一个计划开始编辑"
          description="左侧选择计划查看条目；草稿可直接编辑，已发布计划只读并可克隆为新版本。"
          icon="calendar"
        />
      </div>

      <div v-else-if="loading" class="p-4">
        <div
          v-for="index in 6"
          :key="`entry-skeleton-${index}`"
          class="mb-2 h-8 animate-pulse rounded bg-line/60"
        />
      </div>

      <table v-else-if="sortedEntries.length" class="w-full border-collapse text-left">
        <thead class="sticky top-0 z-10">
          <tr class="border-b border-line bg-canvas/90 backdrop-blur">
            <th class="table-th w-[86px] pl-4">房间</th>
            <th class="table-th w-[92px]">区域</th>
            <th class="table-th w-[52px]">星期</th>
            <th class="table-th w-[112px]">时间</th>
            <th class="table-th w-[104px]">课程</th>
            <th class="table-th w-[76px]">教师</th>
            <th class="table-th w-[88px]">班级</th>
            <th class="table-th">备注</th>
            <th v-if="canManage && isDraft" class="table-th w-[80px] pr-3 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="entry in sortedEntries"
            :key="entry.key"
            class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
          >
            <td class="table-td pl-4">
              <div class="flex flex-col">
                <span class="font-mono text-[10.5px] font-medium text-ink">{{
                  entry.room_code
                }}</span>
                <span
                  class="max-w-[110px] truncate text-[9.5px] text-faint"
                  :title="entry.room_name"
                >
                  {{ entry.room_name }}
                </span>
              </div>
            </td>
            <td class="table-td">
              <span
                class="block max-w-[92px] truncate text-[10.5px] text-muted"
                :title="entry.area_name"
              >
                {{ entry.area_name }}
              </span>
            </td>
            <td class="table-td text-[11px]">
              {{ `周${'一二三四五六日'[entry.day_of_week - 1] ?? '—'}` }}
            </td>
            <td class="table-td font-mono text-[10.5px]">
              {{ entry.start_time }}–{{ entry.end_time }}
            </td>
            <td class="table-td">
              <span
                class="block max-w-[110px] truncate text-[11.5px] font-medium text-ink"
                :title="entry.course_name"
              >
                {{ entry.course_name }}
              </span>
            </td>
            <td class="table-td">
              <span class="block max-w-[80px] truncate text-[11px] text-muted">
                {{ entry.teacher_name || '—' }}
              </span>
            </td>
            <td class="table-td">
              <span class="block max-w-[92px] truncate text-[11px] text-muted">
                {{ entry.class_name || '—' }}
              </span>
            </td>
            <td class="table-td">
              <span
                class="block max-w-[180px] truncate text-[10.5px] text-faint"
                :title="entry.note"
              >
                {{ entry.note || '—' }}
              </span>
            </td>
            <td v-if="canManage && isDraft" class="table-td pr-3 text-right">
              <div class="flex items-center justify-end gap-0.5">
                <IconButton
                  icon="pencil"
                  label="编辑条目"
                  size="sm"
                  @click="emit('editEntry', entry)"
                />
                <IconButton
                  icon="trash"
                  label="移除条目"
                  size="sm"
                  @click="emit('deleteEntry', entry)"
                />
              </div>
            </td>
          </tr>
        </tbody>
      </table>

      <div v-else-if="!error" class="flex h-full items-center justify-center">
        <EmptyState
          :title="isDraft ? '计划还没有条目' : '已发布计划没有条目'"
          :description="
            isDraft
              ? '添加房间、星期、时段与课程，保存草稿并发布后才会写入课表。'
              : '该版本发布时没有条目，可克隆为新版本继续编辑。'
          "
          icon="layers"
        >
          <template v-if="isDraft && canManage" #action>
            <AppButton size="sm" icon="plus" variant="primary" @click="emit('addEntry')">
              添加条目
            </AppButton>
          </template>
        </EmptyState>
      </div>
    </div>

    <footer
      v-if="plan"
      class="flex shrink-0 items-center gap-2 border-t border-line bg-canvas/40 px-4 py-1.5 text-[10px] text-faint"
    >
      <AppIcon name="info" :size="11" />
      <span v-if="isDraft">
        保存草稿只更新计划；点击「发布」才会写入 /schedules 课表并重建排课实例。
      </span>
      <span v-else> 已发布只读：如需调整，克隆为新版本后编辑并再次发布。 </span>
      <span class="ml-auto font-mono">按房间 → 星期 → 时间排序</span>
    </footer>
  </section>
</template>
