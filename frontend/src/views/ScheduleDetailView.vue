<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute } from 'vue-router'

import { api, isForbidden } from '@/api'
import type {
  NewWeeklySlot,
  Occurrence,
  Schedule,
  ScheduleException,
  UpsertExceptionRequest,
  WeeklySlot,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import IconButton from '@/components/ui/IconButton.vue'
import InfoList from '@/components/ui/InfoList.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import OccurrencePanel from '@/features/schedules/OccurrencePanel.vue'
import ScheduleExceptionPanel from '@/features/schedules/ScheduleExceptionPanel.vue'
import ScheduleFormModal from '@/features/schedules/ScheduleFormModal.vue'
import type { ScheduleFormValue } from '@/features/schedules/scheduleForm'
import WeeklySlotEditor from '@/features/schedules/WeeklySlotEditor.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { addDays, formatDateTime, today, toDateInput } from '@/utils/format'
import { errorMessage } from '@/utils/errors'
import { PERMISSIONS } from '@/utils/permissions'

const route = useRoute()
const auth = useAuthStore()
const toast = useToastStore()

const scheduleId = computed(() => String(route.params.id ?? ''))

const schedule = ref<Schedule | null>(null)
const loading = ref(false)
const error = ref('')

const slots = ref<WeeklySlot[]>([])
const slotsLoading = ref(false)
const slotsError = ref('')
const slotsSaving = ref(false)

const exceptions = ref<ScheduleException[]>([])
const exceptionsLoading = ref(false)
const exceptionsError = ref('')
const exceptionSaving = ref(false)
const exceptionDeleting = ref(false)

const occurrences = ref<Occurrence[]>([])
const occurrencesLoading = ref(false)
const occurrencesError = ref('')
const occurrenceFrom = ref(today())
const occurrenceTo = ref(toDateInput(addDays(new Date(), 6)))

const editOpen = ref(false)
const editing = ref(false)
const rebuilding = ref(false)

const canManage = computed(() => auth.can(PERMISSIONS.scheduleManage))

const infoItems = computed(() => {
  const value = schedule.value
  if (!value) return []
  return [
    { label: '房间', value: value.room_name },
    { label: '时区', value: value.timezone, mono: true },
    { label: '课表 ID', value: value.id, mono: true },
    { label: '起止日期', value: `${value.start_date} ~ ${value.end_date}` },
    { label: '创建时间', value: formatDateTime(value.created_at) },
    { label: '更新时间', value: formatDateTime(value.updated_at) },
  ]
})

function actionError(caught: unknown, fallback: string): string {
  return isForbidden(caught) ? '缺少课表管理权限' : errorMessage(caught, fallback)
}

async function loadSchedule() {
  loading.value = true
  error.value = ''
  try {
    schedule.value = await api.getSchedule(scheduleId.value)
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载课表详情失败')
    schedule.value = null
  } finally {
    loading.value = false
  }
}

async function loadSlots() {
  slotsLoading.value = true
  slotsError.value = ''
  try {
    slots.value = await api.weeklySlots(scheduleId.value)
  } catch (caught) {
    slotsError.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载周模板失败')
  } finally {
    slotsLoading.value = false
  }
}

async function loadExceptions() {
  exceptionsLoading.value = true
  exceptionsError.value = ''
  try {
    exceptions.value = await api.exceptions(scheduleId.value)
  } catch (caught) {
    exceptionsError.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载单日覆盖失败')
  } finally {
    exceptionsLoading.value = false
  }
}

async function loadOccurrences() {
  occurrencesLoading.value = true
  occurrencesError.value = ''
  try {
    occurrences.value = await api.occurrences(scheduleId.value, {
      from: occurrenceFrom.value || undefined,
      to: occurrenceTo.value || undefined,
    })
  } catch (caught) {
    occurrencesError.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载排课实例失败')
  } finally {
    occurrencesLoading.value = false
  }
}

async function refreshAll() {
  await loadSchedule()
  if (!schedule.value) return
  await Promise.all([loadSlots(), loadExceptions(), loadOccurrences()])
}

async function submitEdit(value: ScheduleFormValue) {
  editing.value = true
  try {
    schedule.value = await api.updateSchedule(scheduleId.value, {
      name: value.name,
      start_date: value.start_date,
      end_date: value.end_date,
      is_active: value.is_active,
    })
    editOpen.value = false
    toast.success('课表已更新', '未来排课实例已按新配置重建')
    await Promise.all([loadOccurrences(), loadExceptions()])
  } catch (caught) {
    toast.error('更新课表失败', actionError(caught, '请稍后重试'))
  } finally {
    editing.value = false
  }
}

async function rebuild() {
  rebuilding.value = true
  try {
    const result = await api.rebuildSchedule(scheduleId.value)
    toast.success('重建完成', `共生成 ${result.occurrences ?? 0} 个未来排课实例`)
    await loadOccurrences()
  } catch (caught) {
    toast.error('重建排课实例失败', actionError(caught, '请稍后重试'))
  } finally {
    rebuilding.value = false
  }
}

async function saveSlots(next: NewWeeklySlot[]) {
  slotsSaving.value = true
  try {
    slots.value = await api.replaceWeeklySlots(scheduleId.value, next)
    toast.success('周模板已保存', '未来排课实例已重建')
    await loadOccurrences()
  } catch (caught) {
    toast.error('保存周模板失败', actionError(caught, '请检查时段后重试'))
  } finally {
    slotsSaving.value = false
  }
}

async function saveException(date: string, input: UpsertExceptionRequest) {
  exceptionSaving.value = true
  try {
    await api.upsertException(scheduleId.value, date, input)
    toast.success('单日覆盖已保存', `${date} · 未来排课实例已重建`)
    await Promise.all([loadExceptions(), loadOccurrences()])
  } catch (caught) {
    toast.error('保存单日覆盖失败', actionError(caught, '请稍后重试'))
  } finally {
    exceptionSaving.value = false
  }
}

async function deleteException(date: string) {
  exceptionDeleting.value = true
  try {
    await api.deleteException(scheduleId.value, date)
    toast.success('单日覆盖已删除', `${date} · 已按周模板重建`)
    await Promise.all([loadExceptions(), loadOccurrences()])
  } catch (caught) {
    toast.error('删除单日覆盖失败', actionError(caught, '请稍后重试'))
  } finally {
    exceptionDeleting.value = false
  }
}

function queryOccurrences() {
  void loadOccurrences()
}

function openEdit() {
  editOpen.value = true
}

onMounted(() => {
  void refreshAll()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <RouterLink
      :to="{ name: 'schedules' }"
      class="inline-flex items-center gap-1 text-[11.5px] text-muted transition-colors hover:text-ink"
    >
      <AppIcon name="arrow-left" :size="12" />
      返回课表列表
    </RouterLink>

    <PageHeader
      class="mt-2"
      :title="schedule?.name ?? '课表详情'"
      :description="
        schedule
          ? `${schedule.room_name} · ${schedule.timezone} · ${schedule.start_date} ~ ${schedule.end_date}`
          : '周模板、单日覆盖、排课实例与重建。'
      "
      :badge="schedule ? (schedule.is_active ? '启用中' : '已停用') : ''"
    >
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新课表详情"
          variant="outline"
          :loading="loading"
          :disabled="loading"
          @click="refreshAll()"
        />
        <AppButton v-if="canManage" icon="pencil" :disabled="!schedule" @click="openEdit">
          编辑课表
        </AppButton>
        <AppButton
          v-if="canManage"
          icon="refresh"
          variant="primary"
          :loading="rebuilding"
          :disabled="!schedule"
          @click="rebuild"
        >
          手动重建
        </AppButton>
      </template>
    </PageHeader>

    <div
      v-if="error"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <div class="flex items-center gap-2">
        <AppButton size="sm" variant="ghost" @click="refreshAll()">重试</AppButton>
        <RouterLink :to="{ name: 'schedules' }" class="text-[11.5px] text-rose-700 underline">
          返回列表
        </RouterLink>
      </div>
    </div>

    <PageSection
      class="mt-4"
      title="基本信息"
      description="课表归属房间与有效期；编辑后自动重建未来实例。"
    >
      <div class="px-5 py-4">
        <div v-if="loading && !schedule" class="grid grid-cols-2 gap-4">
          <div v-for="index in 6" :key="index" class="h-9 animate-pulse rounded bg-line/60" />
        </div>
        <InfoList v-else :items="infoItems" :columns="2" />
      </div>
    </PageSection>

    <div
      v-if="slotsError"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ slotsError }}</span>
      <AppButton size="sm" variant="ghost" @click="loadSlots()">重试</AppButton>
    </div>

    <WeeklySlotEditor
      class="mt-4"
      :slots="slots"
      :loading="slotsLoading"
      :saving="slotsSaving"
      :can-manage="canManage"
      :timezone="schedule?.timezone ?? ''"
      @reload="loadSlots()"
      @save="saveSlots"
    />

    <ScheduleExceptionPanel
      class="mt-4"
      :exceptions="exceptions"
      :loading="exceptionsLoading"
      :error="exceptionsError"
      :saving="exceptionSaving"
      :deleting="exceptionDeleting"
      :can-manage="canManage"
      @reload="loadExceptions()"
      @save="saveException"
      @delete="deleteException"
    />

    <OccurrencePanel
      class="mt-4"
      v-model:from="occurrenceFrom"
      v-model:to="occurrenceTo"
      :occurrences="occurrences"
      :loading="occurrencesLoading"
      :error="occurrencesError"
      :timezone="schedule?.timezone ?? ''"
      @query="queryOccurrences"
      @reload="loadOccurrences()"
    />

    <p class="mt-3 flex items-center gap-1.5 text-[10.5px] text-faint">
      <StateBadge label="提示" tone="sky" :dot="false" />
      周模板、单日覆盖与课表基本信息的修改都会由后端重建未来的排课实例。
    </p>

    <ScheduleFormModal
      :open="editOpen"
      mode="edit"
      :schedule="schedule"
      :saving="editing"
      @close="editOpen = false"
      @submit="submitEdit"
    />
  </div>
</template>
