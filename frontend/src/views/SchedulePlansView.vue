<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { onBeforeRouteLeave } from 'vue-router'

import { ApiError, api, isForbidden } from '@/api'
import type { Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import MasterDataModal from '@/features/schedulePlans/MasterDataModal.vue'
import MasterDataPanel from '@/features/schedulePlans/MasterDataPanel.vue'
import PlanEditor from '@/features/schedulePlans/PlanEditor.vue'
import PlanEntryModal from '@/features/schedulePlans/PlanEntryModal.vue'
import PlanList from '@/features/schedulePlans/PlanList.vue'
import PlanModal from '@/features/schedulePlans/PlanModal.vue'
import TermBar from '@/features/schedulePlans/TermBar.vue'
import TermModal from '@/features/schedulePlans/TermModal.vue'
import type {
  Course,
  MasterDataType,
  MasterFormValue,
  MasterRecord,
  NewPlanEntry,
  PlanDetail,
  PlanEntryDraft,
  PlanEntryFormValue,
  PlanView,
  SchoolClass,
  Teacher,
  Term,
  TermInput,
} from '@/features/schedulePlans/types'
import {
  draftFromEntry,
  draftFromForm,
  draftToNewEntry,
  entriesSignature,
  formOfDraft,
  masterTypeLabel,
  pickDefaultTerm,
  sortDrafts,
  validateEntryForm,
  validateEntryOverlap,
} from '@/features/schedulePlans/types'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { today } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const auth = useAuthStore()
const toast = useToastStore()

const canManage = computed(() => auth.can(PERMISSIONS.scheduleManage))

/* -------------------------------- 学期 -------------------------------- */

const terms = ref<Term[]>([])
const selectedTermId = ref('')
const termsLoading = ref(false)
const termsError = ref('')

const selectedTerm = computed(
  () => terms.value.find((term) => term.id === selectedTermId.value) ?? null,
)

const termModal = reactive({
  open: false,
  mode: 'create' as 'create' | 'edit',
  saving: false,
  error: '',
})

/* ------------------------------- 主数据 ------------------------------- */

const courses = ref<Course[]>([])
const teachers = ref<Teacher[]>([])
const classes = ref<SchoolClass[]>([])
const masterLoading = ref(false)
const masterError = ref('')
const masterOpen = ref(true)
const masterTab = ref<MasterDataType>('course')

const masterItems = computed<MasterRecord[]>(() => {
  if (masterTab.value === 'course') return courses.value
  if (masterTab.value === 'teacher') return teachers.value
  return classes.value
})

const masterCounts = computed<Record<MasterDataType, number>>(() => ({
  course: courses.value.length,
  teacher: teachers.value.length,
  class: classes.value.length,
}))

const allRecords = computed<MasterRecord[]>(() => [
  ...courses.value,
  ...teachers.value,
  ...classes.value,
])

const masterModal = reactive({
  open: false,
  type: 'course' as MasterDataType,
  record: null as MasterRecord | null,
  saving: false,
  error: '',
})

/* -------------------------------- 房间 -------------------------------- */

const rooms = ref<Room[]>([])
const roomsError = ref('')

/* -------------------------------- 计划 -------------------------------- */

const plans = ref<PlanView[]>([])
const plansLoading = ref(false)
const plansError = ref('')
const selectedPlanId = ref('')

const planDetail = ref<PlanDetail | null>(null)
const planDetailLoading = ref(false)
const planDetailError = ref('')

const drafts = ref<PlanEntryDraft[]>([])
const savedSignature = ref('')

const saving = ref(false)
const publishing = ref(false)
const cloningId = ref('')
const snapshotBusy = computed(() => saving.value || publishing.value || Boolean(cloningId.value))

const selectedPlan = computed<PlanView | null>(
  () =>
    planDetail.value?.plan ?? plans.value.find((plan) => plan.id === selectedPlanId.value) ?? null,
)

const isDraft = computed(() => selectedPlan.value?.status === 'draft')
const dirty = computed(() => entriesSignature(drafts.value) !== savedSignature.value)

const planModal = reactive({ open: false, saving: false, error: '' })

const entryModal = reactive({
  open: false,
  mode: 'create' as 'create' | 'edit',
  key: '' as string | null,
  error: '',
})

const publishBanner = ref<{
  planName: string
  schedules: number
  slots: number
  occurrences: number
} | null>(null)

/* ------------------------------ 确认对话框 ----------------------------- */

const confirmState = reactive({
  open: false,
  title: '',
  description: '',
  detail: '',
  confirmLabel: '确认',
  danger: false,
  loading: false,
  action: null as (() => void | Promise<void>) | null,
})

function askConfirm(config: {
  title: string
  description?: string
  detail?: string
  confirmLabel?: string
  danger?: boolean
  action: () => void | Promise<void>
}) {
  confirmState.open = true
  confirmState.title = config.title
  confirmState.description = config.description ?? ''
  confirmState.detail = config.detail ?? ''
  confirmState.confirmLabel = config.confirmLabel ?? '确认'
  confirmState.danger = config.danger ?? false
  confirmState.loading = false
  confirmState.action = config.action
}

function closeConfirm() {
  if (confirmState.loading) return
  confirmState.open = false
  confirmState.action = null
}

async function runConfirm() {
  const action = confirmState.action
  if (!action) return
  confirmState.loading = true
  try {
    await action()
  } finally {
    confirmState.loading = false
    confirmState.open = false
    confirmState.action = null
  }
}

/* ------------------------------- 工具函数 ------------------------------ */

function actionError(caught: unknown, fallback: string): string {
  return isForbidden(caught) ? '缺少课表管理权限' : errorMessage(caught, fallback)
}

function isConflictError(caught: unknown): boolean {
  return caught instanceof ApiError && caught.status === 409
}

function clearDetail() {
  planDetail.value = null
  drafts.value = []
  savedSignature.value = ''
  planDetailError.value = ''
}

/* -------------------------------- 加载 -------------------------------- */

async function loadTerms(preferId?: string) {
  termsLoading.value = true
  termsError.value = ''
  try {
    const page = await api.listTerms({ page: 1, page_size: 100, include_inactive: true })
    terms.value = page.items ?? []
    const candidate = preferId ?? selectedTermId.value
    if (candidate && terms.value.some((term) => term.id === candidate)) {
      selectedTermId.value = candidate
    } else {
      selectedTermId.value = pickDefaultTerm(terms.value, today())?.id ?? ''
    }
  } catch (caught) {
    termsError.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载学期失败')
    terms.value = []
  } finally {
    termsLoading.value = false
  }
}

async function loadMaster() {
  masterLoading.value = true
  masterError.value = ''
  try {
    const [coursePage, teacherPage, classPage] = await Promise.all([
      api.listCourses({ page: 1, page_size: 100, include_inactive: true }),
      api.listTeachers({ page: 1, page_size: 100, include_inactive: true }),
      api.listClasses({ page: 1, page_size: 100, include_inactive: true }),
    ])
    courses.value = coursePage.items ?? []
    teachers.value = teacherPage.items ?? []
    classes.value = classPage.items ?? []
  } catch (caught) {
    masterError.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载主数据失败')
  } finally {
    masterLoading.value = false
  }
}

async function loadRooms() {
  try {
    const page = await api.listRooms({ page_size: 100 })
    rooms.value = page.items ?? []
    roomsError.value = ''
  } catch (caught) {
    rooms.value = []
    roomsError.value = actionError(caught, '加载房间失败')
  }
}

async function loadPlans() {
  if (!selectedTermId.value) {
    plans.value = []
    plansError.value = ''
    selectedPlanId.value = ''
    clearDetail()
    return
  }
  plansLoading.value = true
  plansError.value = ''
  try {
    const page = await api.listSchedulePlans({
      term_id: selectedTermId.value,
      page_size: 100,
    })
    plans.value = page.items ?? []
    if (!plans.value.some((plan) => plan.id === selectedPlanId.value)) {
      const preferred = plans.value.find((plan) => plan.status === 'draft') ?? plans.value[0]
      selectedPlanId.value = preferred?.id ?? ''
      if (!preferred) clearDetail()
    }
  } catch (caught) {
    plansError.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载计划失败')
    plans.value = []
  } finally {
    plansLoading.value = false
  }
}

async function loadPlanDetail() {
  const planId = selectedPlanId.value
  planDetailError.value = ''
  if (!planId) {
    planDetail.value = null
    drafts.value = []
    savedSignature.value = ''
    planDetailLoading.value = false
    return
  }
  planDetailLoading.value = true
  try {
    const detail = await api.getSchedulePlan(planId)
    planDetail.value = detail
    const list = sortDrafts(detail.entries.map(draftFromEntry))
    drafts.value = list
    savedSignature.value = entriesSignature(list)
    const index = plans.value.findIndex((plan) => plan.id === planId)
    if (index >= 0) plans.value[index] = detail.plan
  } catch (caught) {
    planDetailError.value = isForbidden(caught)
      ? '缺少课表查看权限'
      : errorMessage(caught, '加载计划条目失败')
    planDetail.value = null
    drafts.value = []
    savedSignature.value = ''
  } finally {
    planDetailLoading.value = false
  }
}

/* ------------------------------ 学期操作 ------------------------------ */

function openTermCreate() {
  if (!canManage.value) return
  termModal.mode = 'create'
  termModal.error = ''
  termModal.open = true
}

function openTermEdit() {
  if (!canManage.value || !selectedTerm.value) return
  termModal.mode = 'edit'
  termModal.error = ''
  termModal.open = true
}

async function submitTerm(value: TermInput) {
  const editing = termModal.mode === 'edit' ? selectedTerm.value : null
  termModal.saving = true
  termModal.error = ''
  try {
    if (editing) {
      await api.updateTerm(editing.id, value)
      toast.success('学期已更新', value.name)
      termModal.open = false
      await loadTerms(editing.id)
    } else {
      const created = await api.createTerm(value)
      toast.success('学期已创建', created.name)
      termModal.open = false
      await loadTerms(created.id)
    }
  } catch (caught) {
    termModal.error = isConflictError(caught)
      ? '学期编码已存在，请换一个编码'
      : actionError(caught, '保存学期失败')
  } finally {
    termModal.saving = false
  }
}

function requestDeleteTerm() {
  const term = selectedTerm.value
  if (!term || !canManage.value) return
  askConfirm({
    title: `删除学期「${term.name}」`,
    description: '只有学期下没有任何排课计划时才能删除。',
    detail: '删除后不可恢复；学期下的主数据（课程/教师/班级）不受影响。',
    confirmLabel: '删除学期',
    danger: true,
    action: async () => {
      try {
        await api.deleteTerm(term.id)
        toast.success('学期已删除', term.name)
        if (selectedTermId.value === term.id) selectedTermId.value = ''
        await loadTerms()
      } catch (caught) {
        if (isConflictError(caught)) {
          toast.error('无法删除学期', '该学期下仍有排课计划，请先删除计划')
        } else {
          toast.error('删除学期失败', actionError(caught, '请稍后重试'))
        }
      }
    },
  })
}

function onTermSelect(id: string) {
  if (id === selectedTermId.value) return
  if (dirty.value) {
    toast.info('请先保存草稿或放弃修改', '切换学期会丢弃当前计划条目的未保存修改')
    return
  }
  selectedTermId.value = id
}

/* ------------------------------ 主数据操作 ---------------------------- */

function openMasterCreate() {
  if (!canManage.value) return
  masterModal.type = masterTab.value
  masterModal.record = null
  masterModal.saving = false
  masterModal.error = ''
  masterModal.open = true
}

function openMasterEdit(record: MasterRecord) {
  if (!canManage.value) return
  masterModal.type = masterTab.value
  masterModal.record = record
  masterModal.saving = false
  masterModal.error = ''
  masterModal.open = true
}

async function submitMaster(value: MasterFormValue) {
  const type = masterModal.type
  const record = masterModal.record
  masterModal.saving = true
  masterModal.error = ''
  try {
    if (type === 'course') {
      const input = {
        code: value.code.trim(),
        name: value.name.trim(),
        color: value.color.trim(),
        is_active: value.is_active,
      }
      if (record) await api.updateCourse(record.id, input)
      else await api.createCourse(input)
    } else if (type === 'teacher') {
      const input = {
        code: value.code.trim(),
        name: value.name.trim(),
        phone: value.phone.trim(),
        email: value.email.trim(),
        is_active: value.is_active,
      }
      if (record) await api.updateTeacher(record.id, input)
      else await api.createTeacher(input)
    } else {
      const input = {
        code: value.code.trim(),
        name: value.name.trim(),
        grade: value.grade.trim(),
        is_active: value.is_active,
      }
      if (record) await api.updateClass(record.id, input)
      else await api.createClass(input)
    }
    const label = masterTypeLabel(type)
    toast.success(record ? `${label}已更新` : `${label}已创建`, value.name.trim())
    masterModal.open = false
    await loadMaster()
    if (!dirty.value && selectedPlanId.value) await loadPlanDetail()
  } catch (caught) {
    masterModal.error = isConflictError(caught)
      ? '编码已存在，请换一个编码'
      : actionError(caught, '保存主数据失败')
  } finally {
    masterModal.saving = false
  }
}

function requestDeleteMaster(record: MasterRecord) {
  if (!canManage.value) return
  const label = masterTypeLabel(masterTab.value)
  askConfirm({
    title: `删除${label}「${record.name}」`,
    description: '被计划条目引用的记录不能删除。',
    detail: '该操作不可撤销。',
    confirmLabel: `删除${label}`,
    danger: true,
    action: async () => {
      try {
        if (masterTab.value === 'course') await api.deleteCourse(record.id)
        else if (masterTab.value === 'teacher') await api.deleteTeacher(record.id)
        else await api.deleteClass(record.id)
        toast.success(`${label}已删除`, record.name)
      } catch (caught) {
        if (isConflictError(caught)) {
          toast.error('无法删除', `该${label}被计划条目引用，请先调整相关计划条目`)
        } else {
          toast.error(`删除${label}失败`, actionError(caught, '请稍后重试'))
        }
      } finally {
        await loadMaster()
        if (!dirty.value && selectedPlanId.value) await loadPlanDetail()
      }
    },
  })
}

/* ------------------------------ 计划操作 ------------------------------ */

function openCreatePlan() {
  if (!canManage.value) return
  if (!selectedTermId.value) {
    toast.info('请先创建并选择学期')
    return
  }
  if (dirty.value) {
    toast.info('请先保存草稿或放弃修改', '当前计划条目的未保存修改会丢失')
    return
  }
  planModal.error = ''
  planModal.open = true
}

async function submitPlan(name: string) {
  planModal.saving = true
  planModal.error = ''
  try {
    const created = await api.createSchedulePlan({ term_id: selectedTermId.value, name })
    toast.success('计划已创建', created.name)
    planModal.open = false
    await loadPlans()
    selectedPlanId.value = created.id
  } catch (caught) {
    planModal.error = actionError(caught, '创建计划失败')
  } finally {
    planModal.saving = false
  }
}

function selectPlan(plan: PlanView) {
  if (plan.id === selectedPlanId.value) return
  if (dirty.value) {
    askConfirm({
      title: '放弃未保存的修改？',
      description: `切换到「${plan.name}」会丢弃当前计划条目的未保存修改。`,
      confirmLabel: '放弃并切换',
      danger: true,
      action: () => {
        selectedPlanId.value = plan.id
      },
    })
    return
  }
  selectedPlanId.value = plan.id
}

function requestClonePlan(plan: PlanView) {
  if (!canManage.value) return
  if (dirty.value) {
    toast.info('请先保存草稿或放弃修改', '克隆使用服务端保存的条目')
    return
  }
  askConfirm({
    title: `克隆计划「${plan.name}」`,
    description: '将复制全部条目为新草稿（版本号 +1），不影响已发布版本。',
    confirmLabel: '克隆为新草稿',
    action: async () => {
      cloningId.value = plan.id
      try {
        const created = await api.cloneSchedulePlan(plan.id)
        toast.success('已克隆为新草稿', created.name)
        await loadPlans()
        selectedPlanId.value = created.id
      } catch (caught) {
        toast.error('克隆计划失败', actionError(caught, '请稍后重试'))
      } finally {
        cloningId.value = ''
      }
    },
  })
}

function requestDeletePlan(plan: PlanView) {
  if (!canManage.value || plan.status !== 'draft') return
  askConfirm({
    title: `删除计划「${plan.name}」`,
    description: '删除草稿及其全部条目，不可撤销。',
    detail: '已发布计划不能删除；如需调整可克隆为新版本。',
    confirmLabel: '删除计划',
    danger: true,
    action: async () => {
      try {
        await api.deleteSchedulePlan(plan.id)
        toast.success('计划已删除', plan.name)
        if (selectedPlanId.value === plan.id) {
          selectedPlanId.value = ''
          clearDetail()
        }
        await loadPlans()
      } catch (caught) {
        toast.error(
          '删除计划失败',
          isConflictError(caught) ? '已发布计划不能删除' : actionError(caught, '请稍后重试'),
        )
      }
    },
  })
}

function requestPublishPlan(plan: PlanView) {
  if (!canManage.value || plan.status !== 'draft') return
  askConfirm({
    title: `发布计划「${plan.name}」`,
    description: '发布会把本计划条目写入排课工作台课表，并重建未来排课实例。',
    detail: '注意：会覆盖排课工作台对该学期课表的修改；同一学期其它已发布计划会被降级为草稿。',
    confirmLabel: '确认发布',
    action: () => doPublishPlan(plan),
  })
}

async function doPublishPlan(plan: PlanView) {
  if (plan.id === selectedPlanId.value && dirty.value) {
    const saved = await saveDraft()
    if (!saved) return
  }
  publishing.value = true
  try {
    const result = await api.publishSchedulePlan(plan.id)
    publishBanner.value = {
      planName: result.plan.name,
      schedules: result.schedules_rebuilt,
      slots: result.slots_written,
      occurrences: result.occurrences ?? 0,
    }
    toast.success('已发布', `${result.schedules_rebuilt} 张课表 / ${result.slots_written} 节课程`)
    await Promise.all([loadPlans(), loadPlanDetail()])
  } catch (caught) {
    toast.error(
      '发布失败',
      isConflictError(caught)
        ? '计划已发布或状态不允许，请刷新后重试'
        : actionError(caught, '请稍后重试'),
    )
  } finally {
    publishing.value = false
  }
}

/* ------------------------------ 条目编辑 ------------------------------ */

const entryInitial = computed<PlanEntryFormValue | null>(() => {
  if (!entryModal.open || entryModal.mode !== 'edit') return null
  const draft = drafts.value.find((item) => item.key === entryModal.key)
  return draft ? formOfDraft(draft) : null
})

function openEntryCreate() {
  if (!canManage.value || !isDraft.value) return
  if (!rooms.value.length) {
    toast.info('没有可用房间', roomsError.value || '请先在「房间」页面创建房间')
    return
  }
  if (!courses.value.length) {
    toast.info('还没有课程主数据', '请先在「课程」tab 创建课程')
    return
  }
  entryModal.mode = 'create'
  entryModal.key = null
  entryModal.error = ''
  entryModal.open = true
}

function openEntryEdit(entry: PlanEntryDraft) {
  if (!canManage.value || !isDraft.value) return
  entryModal.mode = 'edit'
  entryModal.key = entry.key
  entryModal.error = ''
  entryModal.open = true
}

function submitEntry(value: PlanEntryFormValue) {
  const message = validateEntryForm(value)
  if (message) {
    entryModal.error = message
    return
  }
  const overlap = validateEntryOverlap(
    drafts.value,
    value,
    entryModal.mode === 'edit' ? (entryModal.key ?? undefined) : undefined,
  )
  if (overlap) {
    entryModal.error = overlap
    return
  }
  const next = draftFromForm(value, rooms.value, allRecords.value)
  if (entryModal.mode === 'edit' && entryModal.key) {
    const key = entryModal.key
    drafts.value = sortDrafts(
      drafts.value.map((draft) =>
        draft.key === key ? { ...next, key: draft.key, id: draft.id } : draft,
      ),
    )
  } else {
    drafts.value = sortDrafts([...drafts.value, next])
  }
  entryModal.open = false
  entryModal.error = ''
}

function deleteEntry(entry: PlanEntryDraft) {
  if (!canManage.value || !isDraft.value) return
  drafts.value = drafts.value.filter((draft) => draft.key !== entry.key)
}

function discardDrafts() {
  if (!planDetail.value) return
  const list = sortDrafts(planDetail.value.entries.map(draftFromEntry))
  drafts.value = list
  savedSignature.value = entriesSignature(list)
  toast.info('已放弃未保存修改')
}

async function saveDraft(): Promise<boolean> {
  const plan = selectedPlan.value
  if (!plan || plan.status !== 'draft' || !canManage.value) return false
  if (!dirty.value) {
    toast.info('没有需要保存的修改')
    return true
  }
  saving.value = true
  try {
    await api.replaceSchedulePlanEntries(
      plan.id,
      drafts.value.map((draft) => draftToNewEntry(draft) as NewPlanEntry),
    )
    toast.success('草稿已保存', `${plan.name} · ${drafts.value.length} 条条目`)
    await Promise.all([loadPlanDetail(), loadPlans()])
    return true
  } catch (caught) {
    toast.error(
      '保存草稿失败',
      isConflictError(caught)
        ? '条目存在冲突或计划不是草稿'
        : actionError(caught, '请检查条目后重试'),
    )
    return false
  } finally {
    saving.value = false
  }
}

function publishSelected() {
  if (selectedPlan.value) requestPublishPlan(selectedPlan.value)
}

function cloneSelected() {
  if (selectedPlan.value) requestClonePlan(selectedPlan.value)
}

/* ------------------------------- 生命周期 ----------------------------- */

watch(selectedTermId, () => {
  void loadPlans()
})

watch(selectedPlanId, () => {
  void loadPlanDetail()
})

onBeforeRouteLeave(() => {
  if (!dirty.value) return true
  return window.confirm('当前排课计划有未保存的条目修改，确定离开？')
})

onMounted(() => {
  void Promise.all([loadTerms(), loadMaster(), loadRooms()])
})
</script>

<template>
  <div class="flex h-full min-h-0 flex-col px-7 py-5">
    <div class="mx-auto flex min-h-0 w-full max-w-[1440px] flex-1 flex-col">
      <PageHeader
        title="排课计划"
        description="按学期维护课程 / 教师 / 班级主数据与排课计划；草稿保存后发布，才会写入排课工作台课表。"
        :badge="canManage ? '' : '只读'"
      >
        <template #actions>
          <RouterLink
            :to="{ name: 'schedules' }"
            class="inline-flex h-9 items-center gap-1.5 rounded-md border border-line bg-surface px-3 text-[12.5px] text-ink-soft transition-colors hover:border-line-strong hover:text-ink"
          >
            <AppIcon name="calendar" :size="13" />
            去排课工作台
          </RouterLink>
          <AppButton
            v-if="canManage"
            icon="plus"
            variant="primary"
            :disabled="!selectedTermId"
            @click="openCreatePlan"
          >
            新建计划
          </AppButton>
        </template>
      </PageHeader>

      <div
        v-if="termsError"
        class="mt-3 flex shrink-0 items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2 text-[11.5px] text-rose-700"
      >
        <span class="flex items-center gap-1.5">
          <AppIcon name="alert" :size="13" />
          {{ termsError }}
        </span>
        <AppButton size="sm" variant="ghost" @click="loadTerms()">重试</AppButton>
      </div>

      <TermBar
        :terms="terms"
        :selected-term-id="selectedTermId"
        :loading="termsLoading"
        :error="termsError"
        :can-manage="canManage"
        :dirty="dirty"
        :plan-count="plans.length"
        @update:selected-term-id="onTermSelect"
        @create="openTermCreate"
        @edit="openTermEdit"
        @delete="requestDeleteTerm"
        @retry="loadTerms()"
      />

      <div
        v-if="publishBanner"
        class="mt-3 flex shrink-0 items-center gap-2 rounded-lg border border-emerald-200 bg-emerald-50 px-3.5 py-2 text-[11.5px] text-emerald-800"
      >
        <AppIcon name="check-circle" :size="14" />
        <span class="min-w-0 truncate">
          已发布「{{ publishBanner.planName }}」· {{ publishBanner.schedules }} 张课表 /
          {{ publishBanner.slots }} 节课程，排课实例已重建 {{ publishBanner.occurrences }} 个
        </span>
        <RouterLink
          :to="{ name: 'schedules' }"
          class="ml-auto inline-flex shrink-0 items-center gap-1 font-medium text-emerald-700 hover:underline"
        >
          去排课工作台查看
          <AppIcon name="arrow-right" :size="12" />
        </RouterLink>
        <button
          type="button"
          aria-label="关闭发布提示"
          class="flex h-6 w-6 shrink-0 items-center justify-center rounded text-emerald-700/70 hover:bg-emerald-100"
          @click="publishBanner = null"
        >
          <AppIcon name="x" :size="13" />
        </button>
      </div>

      <MasterDataPanel
        v-model:open="masterOpen"
        v-model:type="masterTab"
        :items="masterItems"
        :counts="masterCounts"
        :loading="masterLoading"
        :error="masterError"
        :can-manage="canManage"
        @create="openMasterCreate"
        @edit="openMasterEdit"
        @delete="requestDeleteMaster"
        @retry="loadMaster()"
      />

      <div class="mt-3 grid min-h-0 flex-1 grid-cols-[340px_minmax(0,1fr)] gap-3">
        <PlanList
          :plans="plans"
          :selected-id="selectedPlanId"
          :loading="plansLoading"
          :error="plansError"
          :can-manage="canManage"
          @select="selectPlan"
          @create="openCreatePlan"
          @clone="requestClonePlan"
          @delete="requestDeletePlan"
          @publish="requestPublishPlan"
          @retry="loadPlans()"
        />

        <PlanEditor
          :plan="selectedPlan"
          :entries="drafts"
          :term-name="selectedTerm?.name ?? ''"
          :loading="planDetailLoading && !!selectedPlanId"
          :error="planDetailError"
          :can-manage="canManage"
          :dirty="dirty"
          :saving="saving"
          :publishing="publishing"
          :cloning="Boolean(cloningId)"
          @add-entry="openEntryCreate"
          @edit-entry="openEntryEdit"
          @delete-entry="deleteEntry"
          @save="saveDraft()"
          @publish="publishSelected"
          @clone="cloneSelected"
          @retry="loadPlanDetail()"
          @discard="discardDrafts"
        />
      </div>
    </div>

    <TermModal
      :open="termModal.open"
      :mode="termModal.mode"
      :term="termModal.mode === 'edit' ? selectedTerm : null"
      :saving="termModal.saving"
      :error="termModal.error"
      @close="termModal.open = false"
      @submit="submitTerm"
    />

    <MasterDataModal
      :open="masterModal.open"
      :type="masterModal.type"
      :record="masterModal.record"
      :saving="masterModal.saving"
      :error="masterModal.error"
      @close="masterModal.open = false"
      @submit="submitMaster"
    />

    <PlanModal
      :open="planModal.open"
      :term-name="selectedTerm?.name ?? ''"
      :saving="planModal.saving"
      :error="planModal.error"
      @close="planModal.open = false"
      @submit="submitPlan"
    />

    <PlanEntryModal
      :open="entryModal.open"
      :mode="entryModal.mode"
      :rooms="rooms"
      :courses="courses"
      :teachers="teachers"
      :classes="classes"
      :initial="entryInitial"
      :error="entryModal.error"
      @close="entryModal.open = false"
      @submit="submitEntry"
    />

    <ConfirmDialog
      :open="confirmState.open"
      :title="confirmState.title"
      :description="confirmState.description"
      :detail="confirmState.detail"
      :confirm-label="confirmState.confirmLabel"
      :danger="confirmState.danger"
      :loading="confirmState.loading"
      @confirm="runConfirm"
      @close="closeConfirm"
    />

    <p
      v-if="snapshotBusy"
      class="pointer-events-none fixed bottom-10 right-6 z-40 flex items-center gap-1.5 rounded-md border border-line bg-surface px-3 py-1.5 text-[10.5px] text-muted shadow-sm"
    >
      <AppIcon name="refresh" :size="12" class="animate-spin" />
      正在同步计划…
    </p>
  </div>
</template>
