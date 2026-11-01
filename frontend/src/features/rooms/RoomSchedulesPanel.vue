<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type { Occurrence, Schedule } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { errorMessage } from '@/utils/errors'
import { formatDateTime, shortId } from '@/utils/format'

const props = defineProps<{
  roomId: string
}>()

const schedules = ref<Schedule[]>([])
const occurrences = ref<Occurrence[]>([])
const loading = ref(false)
const error = ref('')

const scheduleNames = computed(() => {
  const map = new Map<string, string>()
  for (const schedule of schedules.value) map.set(schedule.id, schedule.name)
  return map
})

const sourceLabels: Record<'weekly' | 'custom', string> = {
  weekly: '周课表',
  custom: '自定义',
}

function occurrenceDate(occurrence: Occurrence): string {
  return occurrence.occurrence_date ?? occurrence.start_local?.slice(0, 10) ?? '—'
}

function occurrenceTime(occurrence: Occurrence): string {
  const start = occurrence.start_local?.slice(11, 16)
  const end = occurrence.end_local?.slice(11, 16)
  if (start && end) return `${start}–${end}`
  if (occurrence.start_at && occurrence.end_at) {
    return `${formatDateTime(occurrence.start_at)} – ${formatDateTime(occurrence.end_at)}`
  }
  return '—'
}

function scheduleName(scheduleId: string | undefined): string {
  if (!scheduleId) return '—'
  return scheduleNames.value.get(scheduleId) ?? shortId(scheduleId)
}

async function load() {
  if (!props.roomId) return
  loading.value = true
  error.value = ''
  try {
    const [schedulePage, occurrenceList] = await Promise.all([
      api.listSchedules({ room_id: props.roomId, page_size: 100 }),
      api.roomOccurrences(props.roomId),
    ])
    schedules.value = schedulePage.items ?? []
    occurrences.value = [...occurrenceList].sort((left, right) =>
      (left.start_at ?? '').localeCompare(right.start_at ?? ''),
    )
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少权限：当前账号没有课表读取权限（schedule:read）。'
      : errorMessage(caught, '加载房间课表失败')
  } finally {
    loading.value = false
  }
}

watch(
  () => props.roomId,
  () => void load(),
)
onMounted(() => void load())
</script>

<template>
  <PageSection>
    <template #header>
      <div class="flex items-start justify-between gap-3">
        <div>
          <h2 class="text-[13.5px] font-semibold text-ink">课表与近期实例</h2>
          <p class="mt-0.5 text-[11px] text-muted">
            该房间的课表，以及按房间时区今天起 7 天的排课实例。
          </p>
        </div>
        <IconButton
          icon="refresh"
          label="刷新课表与实例"
          variant="outline"
          :loading="loading"
          @click="load"
        />
      </div>
    </template>

    <div class="space-y-4 px-5 py-4">
      <div
        v-if="error"
        role="alert"
        class="flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
      >
        <span>{{ error }}</span>
        <AppButton size="sm" variant="ghost" @click="load">重试</AppButton>
      </div>

      <div v-if="loading" class="space-y-2">
        <div class="h-4 w-2/3 animate-pulse rounded bg-canvas" />
        <div class="h-4 w-1/2 animate-pulse rounded bg-canvas" />
        <div class="h-4 w-3/5 animate-pulse rounded bg-canvas" />
      </div>

      <template v-else>
        <div>
          <h3 class="text-[11.5px] font-medium text-ink-soft">
            课表
            <span class="ml-1 font-mono text-[10px] text-faint">{{ schedules.length }}</span>
          </h3>
          <ul v-if="schedules.length" class="mt-2 divide-y divide-line/70">
            <li v-for="schedule in schedules" :key="schedule.id">
              <RouterLink
                :to="{ name: 'schedule-detail', params: { id: schedule.id } }"
                class="flex items-center gap-3 rounded px-1 py-2 transition-colors hover:bg-canvas/60"
              >
                <span class="min-w-0 flex-1 truncate text-[12px] font-medium text-ink">
                  {{ schedule.name }}
                </span>
                <span class="shrink-0 font-mono text-[10.5px] text-faint">
                  {{ schedule.start_date }} ~ {{ schedule.end_date }}
                </span>
                <StateBadge
                  :label="schedule.is_active ? '启用' : '已停用'"
                  :tone="schedule.is_active ? 'emerald' : 'slate'"
                />
                <span class="shrink-0 font-mono text-[10px] text-faint">
                  {{ schedule.timezone }}
                </span>
              </RouterLink>
            </li>
          </ul>
          <p v-else class="mt-2 text-[11.5px] text-muted">该房间还没有课表。</p>
        </div>

        <div class="border-t border-line/70 pt-3.5">
          <h3 class="text-[11.5px] font-medium text-ink-soft">
            近 7 天排课实例
            <span class="ml-1 font-mono text-[10px] text-faint">{{ occurrences.length }}</span>
          </h3>
          <ul v-if="occurrences.length" class="mt-2 divide-y divide-line/70">
            <li
              v-for="occurrence in occurrences"
              :key="occurrence.id ?? `${occurrence.occurrence_date}-${occurrence.start_local}`"
              class="flex items-center gap-3 px-1 py-2"
            >
              <span class="w-[86px] shrink-0 font-mono text-[10.5px] text-ink-soft">
                {{ occurrenceDate(occurrence) }}
              </span>
              <span class="w-[88px] shrink-0 font-mono text-[10.5px] text-muted">
                {{ occurrenceTime(occurrence) }}
              </span>
              <span class="min-w-0 flex-1 truncate text-[12px] text-ink">
                {{ occurrence.course_name || '未命名课程' }}
                <span v-if="occurrence.teacher" class="text-muted">
                  · {{ occurrence.teacher }}
                </span>
              </span>
              <StateBadge
                v-if="occurrence.source"
                :label="sourceLabels[occurrence.source]"
                tone="sky"
                :dot="false"
              />
              <span class="shrink-0 text-[10.5px] text-faint">
                {{ scheduleName(occurrence.schedule_id) }}
              </span>
            </li>
          </ul>
          <p v-else class="mt-2 text-[11.5px] text-muted">该房间时区今天起 7 天内没有排课实例。</p>
        </div>
      </template>
    </div>
  </PageSection>
</template>
