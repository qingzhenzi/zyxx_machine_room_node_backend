<script setup lang="ts">
/**
 * 巡检计划新建 / 编辑弹窗（P1 Wave 3）。
 * 范围 global/area/room + 间隔小时（1–168）+ 启用开关；范围名称由页面 best-effort 注入。
 */
import { computed, ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type {
  CreateInspectionPlanRequest,
  InspectionPlanView,
  UpdateInspectionPlanRequest,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'

import {
  hasInspectionErrors,
  INSPECTION_INTERVAL_DEFAULT,
  INSPECTION_INTERVAL_MAX,
  INSPECTION_INTERVAL_MIN,
  INSPECTION_NAME_MAX_LENGTH,
  INSPECTION_SCOPE_META,
  INSPECTION_SCOPE_ORDER,
  normalizeInspectionScope,
  validateInspectionDraft,
  type InspectionDraftErrors,
  type InspectionScopeType,
} from './inspectionMeta'

interface ScopeOption {
  id: string
  name: string
}

const props = withDefaults(
  defineProps<{
    open: boolean
    plan: InspectionPlanView | null
    areas?: ScopeOption[]
    rooms?: ScopeOption[]
  }>(),
  { areas: () => [], rooms: () => [] },
)

const emit = defineEmits<{
  close: []
  saved: []
}>()

const toast = useToastStore()

const name = ref('')
const scopeType = ref<InspectionScopeType>('global')
const scopeId = ref('')
const intervalHours = ref<number | string>(INSPECTION_INTERVAL_DEFAULT)
const enabled = ref(true)
const errors = ref<InspectionDraftErrors>({})
const formError = ref('')
const submitting = ref(false)

const isEdit = computed(() => props.plan !== null)
const modalTitle = computed(() => (isEdit.value ? '编辑巡检计划' : '新建巡检计划'))
const availableScopeOptions = computed(() => {
  if (scopeType.value === 'area') return props.areas
  if (scopeType.value === 'room') return props.rooms
  return []
})
const scopeMeta = computed(() => INSPECTION_SCOPE_META[scopeType.value])

function resetForm() {
  const plan = props.plan
  if (plan) {
    name.value = plan.name
    scopeType.value = normalizeInspectionScope(plan.scope_type)
    scopeId.value = plan.scope_id ?? ''
    intervalHours.value = plan.interval_hours
    enabled.value = plan.enabled
  } else {
    name.value = ''
    scopeType.value = 'global'
    scopeId.value = ''
    intervalHours.value = INSPECTION_INTERVAL_DEFAULT
    enabled.value = true
  }
  errors.value = {}
  formError.value = ''
}

watch(
  () => props.open,
  (open) => {
    if (open) resetForm()
  },
)

watch(scopeType, () => {
  scopeId.value = ''
})

function presetInterval(hours: number) {
  intervalHours.value = hours
}

async function submit() {
  const draft = {
    name: name.value,
    scope_type: scopeType.value,
    scope_id: scopeType.value === 'global' ? '' : scopeId.value,
    interval_hours: intervalHours.value,
    enabled: enabled.value,
  }
  const nextErrors = validateInspectionDraft(draft)
  errors.value = nextErrors
  formError.value = ''
  if (hasInspectionErrors(nextErrors)) return

  const common = {
    name: name.value.trim(),
    scope_type: scopeType.value,
    scope_id: scopeType.value === 'global' ? null : scopeId.value,
    interval_hours: Number(intervalHours.value),
    enabled: enabled.value,
  }

  submitting.value = true
  try {
    if (props.plan) {
      const saved = await api.updateInspectionPlan(
        props.plan.id,
        common as UpdateInspectionPlanRequest,
      )
      toast.success('巡检计划已更新', saved.name)
    } else {
      const saved = await api.createInspectionPlan(common as CreateInspectionPlanRequest)
      toast.success('巡检计划已创建', saved.name)
    }
    emit('saved')
    emit('close')
  } catch (caught) {
    if (isForbidden(caught)) {
      formError.value = '缺少权限：需要 ticket:inspection:manage 或 ticket:manage。'
    } else {
      formError.value = errorMessage(caught, '保存巡检计划失败，请稍后重试')
    }
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="props.open"
    :title="modalTitle"
    description="worker 按间隔巡检范围内机器的最新健康快照；发现异常时自动建一张机器健康工单。"
    size="lg"
    @close="emit('close')"
  >
    <form class="flex flex-col gap-4" @submit.prevent="submit">
      <FormField label="计划名称" input-id="inspection-name" required :error="errors.name">
        <input
          id="inspection-name"
          v-model="name"
          class="input"
          :maxlength="INSPECTION_NAME_MAX_LENGTH"
          placeholder="例如：每日机房巡检"
          data-autofocus
        />
      </FormField>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="巡检范围" input-id="inspection-scope" :hint="scopeMeta.description">
          <select id="inspection-scope" v-model="scopeType" class="select">
            <option v-for="key in INSPECTION_SCOPE_ORDER" :key="key" :value="key">
              {{ INSPECTION_SCOPE_META[key].label }}
            </option>
          </select>
        </FormField>

        <FormField
          v-if="scopeType !== 'global'"
          :label="scopeType === 'area' ? '区域' : '房间'"
          input-id="inspection-scope-id"
          :error="errors.scope_id"
          :hint="
            availableScopeOptions.length
              ? undefined
              : '当前账号没有区域 / 房间读取权限，可直接填写 UUID'
          "
        >
          <select
            v-if="availableScopeOptions.length"
            id="inspection-scope-id"
            v-model="scopeId"
            class="select"
          >
            <option value="">请选择</option>
            <option v-for="option in availableScopeOptions" :key="option.id" :value="option.id">
              {{ option.name }}
            </option>
          </select>
          <input
            v-else
            id="inspection-scope-id"
            v-model="scopeId"
            class="input font-mono"
            placeholder="scope UUID"
          />
        </FormField>
      </div>

      <FormField
        label="巡检间隔"
        input-id="inspection-interval"
        :error="errors.interval_hours"
        :hint="`${INSPECTION_INTERVAL_MIN}–${INSPECTION_INTERVAL_MAX} 小时；worker 按该间隔自动执行。`"
      >
        <div class="flex items-center gap-2">
          <input
            id="inspection-interval"
            v-model="intervalHours"
            class="input"
            type="number"
            :min="INSPECTION_INTERVAL_MIN"
            :max="INSPECTION_INTERVAL_MAX"
            step="1"
          />
          <span class="shrink-0 text-[11px] text-faint">小时</span>
        </div>
        <div class="mt-2 flex flex-wrap gap-1.5">
          <button
            v-for="preset in [6, 12, 24, 72, 168]"
            :key="preset"
            type="button"
            class="rounded border border-line bg-surface px-2 py-1 text-[10.5px] text-muted transition-colors hover:border-line-strong hover:text-ink"
            :class="
              Number(intervalHours) === preset
                ? 'border-accent-line bg-accent-soft text-accent'
                : ''
            "
            @click="presetInterval(preset)"
          >
            {{ preset }}h
          </button>
        </div>
      </FormField>

      <div class="flex items-center justify-between rounded-md border border-line px-4 py-3">
        <div>
          <p class="text-[11.5px] font-medium text-ink-soft">启用计划</p>
          <p class="mt-0.5 text-[10.5px] text-faint">
            停用后 worker 不再自动执行，仍可手动「立即运行」。
          </p>
        </div>
        <ToggleSwitch v-model="enabled" />
      </div>

      <p
        v-if="formError"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
      >
        {{ formError }}
      </p>

      <div class="flex items-center justify-end gap-2 border-t border-line pt-4">
        <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
        <AppButton type="submit" variant="primary" :loading="submitting">
          {{ isEdit ? '保存计划' : '创建计划' }}
        </AppButton>
      </div>
    </form>
  </AppModal>
</template>
