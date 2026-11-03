<script setup lang="ts">
import { computed } from 'vue'

import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { formatRelative } from '@/utils/format'

import type { PlanView } from './types'
import { planStatusMeta } from './types'

const props = withDefaults(
  defineProps<{
    plans: PlanView[]
    selectedId: string
    loading?: boolean
    error?: string
    canManage?: boolean
  }>(),
  { loading: false, error: '', canManage: false },
)

const emit = defineEmits<{
  select: [plan: PlanView]
  create: []
  clone: [plan: PlanView]
  delete: [plan: PlanView]
  publish: [plan: PlanView]
  retry: []
}>()

const drafts = computed(() => props.plans.filter((plan) => plan.status === 'draft').length)
const published = computed(() => props.plans.filter((plan) => plan.status === 'published').length)

function meta(plan: PlanView) {
  return planStatusMeta(plan.status)
}
</script>

<template>
  <section class="flex min-h-0 flex-col overflow-hidden rounded-xl border border-line bg-surface">
    <header
      class="flex shrink-0 items-center justify-between gap-2 border-b border-line px-3.5 py-2.5"
    >
      <div class="min-w-0">
        <h2 class="text-[13px] font-semibold text-ink">计划</h2>
        <p class="mt-0.5 text-[10.5px] text-faint">
          共 {{ plans.length }} 个 · 草稿 {{ drafts }} · 已发布 {{ published }}
        </p>
      </div>
      <div class="flex shrink-0 items-center gap-1">
        <IconButton
          icon="refresh"
          label="刷新计划列表"
          variant="outline"
          :loading="loading"
          :disabled="loading"
          @click="emit('retry')"
        />
      </div>
    </header>

    <div
      v-if="error"
      class="flex shrink-0 items-center justify-between border-b border-rose-200 bg-rose-50 px-3.5 py-2 text-[11px] text-rose-700"
    >
      <span class="truncate">{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="emit('retry')">重试</AppButton>
    </div>

    <div class="min-h-0 flex-1 overflow-y-auto">
      <div v-if="loading && !plans.length" class="p-3">
        <div
          v-for="index in 4"
          :key="`plan-skeleton-${index}`"
          class="mb-2 h-14 animate-pulse rounded-md bg-line/60"
        />
      </div>

      <ul v-else-if="plans.length" class="divide-y divide-line/70">
        <li v-for="plan in plans" :key="plan.id">
          <div
            role="button"
            tabindex="0"
            class="group w-full cursor-pointer px-3.5 py-2.5 text-left transition-colors"
            :class="plan.id === selectedId ? 'bg-accent-soft/60' : 'hover:bg-canvas/60'"
            :aria-current="plan.id === selectedId ? 'true' : undefined"
            @click="emit('select', plan)"
            @keydown.enter.prevent="emit('select', plan)"
          >
            <div class="flex items-center gap-1.5">
              <span class="min-w-0 flex-1 truncate text-[12px] font-medium text-ink">
                {{ plan.name }}
              </span>
              <StateBadge :label="meta(plan).label" :tone="meta(plan).tone" />
              <span
                class="shrink-0 rounded border border-line px-1 font-mono text-[9px] text-faint"
              >
                v{{ plan.version }}
              </span>
            </div>
            <div class="mt-1 flex items-center gap-2 text-[10px] text-faint">
              <span>{{ plan.entry_count }} 条目</span>
              <span class="h-2.5 w-px bg-line" />
              <span>{{ plan.room_count }} 房间</span>
              <span class="h-2.5 w-px bg-line" />
              <span>{{ plan.course_count }} 课程</span>
            </div>
            <div class="mt-0.5 flex items-center gap-1 text-[9.5px] text-faint">
              <AppIcon name="clock" :size="10" />
              <span v-if="plan.status === 'published' && plan.published_at">
                发布于 {{ formatRelative(plan.published_at) }}
              </span>
              <span v-else>更新于 {{ formatRelative(plan.updated_at) }}</span>
            </div>
          </div>

          <div
            class="flex items-center justify-between border-t border-line/60 bg-canvas/40 px-2 py-1"
          >
            <span class="pl-1 text-[9.5px] text-faint">
              {{ plan.status === 'published' ? '已发布只读 · 可克隆新版本' : '草稿可编辑条目' }}
            </span>
            <div class="flex items-center gap-0.5">
              <IconButton
                icon="arrow-right"
                label="打开计划编辑器"
                size="sm"
                @click.stop="emit('select', plan)"
              />
              <template v-if="canManage">
                <IconButton
                  v-if="plan.status === 'draft'"
                  icon="play-circle"
                  label="发布计划"
                  size="sm"
                  @click.stop="emit('publish', plan)"
                />
                <IconButton
                  icon="copy"
                  label="克隆为新草稿"
                  size="sm"
                  @click.stop="emit('clone', plan)"
                />
                <IconButton
                  v-if="plan.status === 'draft'"
                  icon="trash"
                  label="删除计划"
                  size="sm"
                  @click.stop="emit('delete', plan)"
                />
              </template>
            </div>
          </div>
        </li>
      </ul>

      <EmptyState
        v-else-if="!error"
        title="该学期还没有计划"
        description="新建计划后逐条添加课程条目；草稿保存不影响课表，发布后才写入排课工作台。"
        icon="calendar"
      >
        <template v-if="canManage" #action>
          <AppButton size="sm" icon="plus" variant="primary" @click="emit('create')">
            新建计划
          </AppButton>
        </template>
      </EmptyState>
    </div>
  </section>
</template>
