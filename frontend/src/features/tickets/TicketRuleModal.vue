<script setup lang="ts">
/**
 * 自动建单规则新建 / 编辑弹窗（P1 Wave 1）。
 * 阈值按 signal_type 动态渲染；编辑时信号类型不可改（后端只允许创建时指定）。
 */
import { computed, ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type {
  CreateTicketRuleRequest,
  TicketPriority,
  TicketRule,
  UpdateTicketRuleRequest,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'

import {
  buildThresholdPayload,
  defaultThresholds,
  normalizeScopeType,
  normalizeSignalType,
  normalizeThresholds,
  TICKET_RULE_SCOPE_META,
  TICKET_RULE_SCOPE_ORDER,
  TICKET_RULE_SIGNAL_META,
  TICKET_RULE_SIGNAL_ORDER,
  thresholdFields,
  validateRuleDraft,
  hasRuleErrors,
  type RuleDraftErrors,
  type TicketRuleScopeType,
  type TicketRuleSignalType,
} from './ticketRuleMeta'
import { TICKET_PRIORITY_ORDER, ticketPriorityMeta } from './ticketMeta'

interface ScopeOption {
  id: string
  name: string
}

const props = withDefaults(
  defineProps<{
    open: boolean
    rule: TicketRule | null
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
const signalType = ref<TicketRuleSignalType>('machine_health')
const priority = ref<TicketPriority>('normal')
const scopeType = ref<TicketRuleScopeType>('global')
const scopeId = ref('')
const enabled = ref(true)
const thresholds = ref<Record<string, string>>({})
const errors = ref<RuleDraftErrors>({})
const formError = ref('')
const submitting = ref(false)

const isEdit = computed(() => props.rule !== null)
const modalTitle = computed(() => (isEdit.value ? '编辑自动规则' : '新建自动规则'))
const fields = computed(() => thresholdFields(signalType.value))
const scopeMeta = computed(() => TICKET_RULE_SCOPE_META[scopeType.value])
/** 镜像验证是全局信号，只允许 global 范围。 */
const scopeSupported = computed(() => signalType.value !== 'image_verify')

function resetForm() {
  const rule = props.rule
  if (rule) {
    name.value = rule.name
    signalType.value = normalizeSignalType(rule.signal_type) ?? 'machine_health'
    priority.value = rule.priority
    scopeType.value = normalizeScopeType(rule.scope_type)
    scopeId.value = rule.scope_id ?? ''
    enabled.value = rule.enabled
    const normalized = normalizeThresholds(rule.signal_type, rule.thresholds)
    const next: Record<string, string> = {}
    for (const field of thresholdFields(rule.signal_type))
      next[field.key] = String(normalized[field.key])
    thresholds.value = next
  } else {
    name.value = ''
    signalType.value = 'machine_health'
    priority.value = 'normal'
    scopeType.value = 'global'
    scopeId.value = ''
    enabled.value = true
    const defaults = defaultThresholds(signalType.value)
    const next: Record<string, string> = {}
    for (const key of Object.keys(defaults)) next[key] = String(defaults[key])
    thresholds.value = next
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

watch(signalType, (value) => {
  if (value === 'image_verify') {
    scopeType.value = 'global'
    scopeId.value = ''
  }
  if (isEdit.value) return
  const defaults = defaultThresholds(value)
  const next: Record<string, string> = {}
  for (const key of Object.keys(defaults)) next[key] = String(defaults[key])
  thresholds.value = next
  errors.value = {}
})

watch(scopeType, () => {
  scopeId.value = ''
})

function scopeOptions(): ScopeOption[] {
  return scopeType.value === 'area' ? props.areas : scopeType.value === 'room' ? props.rooms : []
}

const availableScopeOptions = computed(() => scopeOptions())

async function submit() {
  const draft = {
    name: name.value,
    signal_type: signalType.value,
    priority: priority.value,
    scope_type: scopeType.value,
    scope_id: scopeType.value === 'global' ? '' : scopeId.value,
    enabled: enabled.value,
    thresholds: thresholds.value,
  }
  const nextErrors = validateRuleDraft(draft)
  errors.value = nextErrors
  formError.value = ''
  if (hasRuleErrors(nextErrors)) return

  const common = {
    name: name.value.trim(),
    enabled: enabled.value,
    priority: priority.value,
    scope_type: scopeType.value,
    scope_id: scopeType.value === 'global' ? null : scopeId.value,
    thresholds: buildThresholdPayload(signalType.value, thresholds.value),
  }

  submitting.value = true
  try {
    if (props.rule) {
      const payload = common as UpdateTicketRuleRequest
      const saved = await api.updateTicketRule(props.rule.id, payload)
      toast.success('自动规则已更新', saved.name)
    } else {
      const payload: CreateTicketRuleRequest = {
        ...common,
        signal_type: signalType.value,
      }
      const saved = await api.createTicketRule(payload)
      toast.success('自动规则已创建', saved.name)
    }
    emit('saved')
    emit('close')
  } catch (caught) {
    if (isForbidden(caught)) {
      formError.value = '缺少权限：需要 ticket:rule:manage 或 ticket:manage。'
    } else {
      formError.value = errorMessage(caught, '保存自动规则失败，请稍后重试')
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
    description="自动规则决定 worker 在哪些信号下自动建单；停用后该信号不再建单。"
    size="lg"
    @close="emit('close')"
  >
    <form class="flex flex-col gap-4" @submit.prevent="submit">
      <FormField label="规则名称" input-id="rule-name" required :error="errors.name">
        <input
          id="rule-name"
          v-model="name"
          class="input"
          maxlength="120"
          placeholder="例如：机器健康连续异常"
          data-autofocus
        />
      </FormField>

      <div class="grid grid-cols-2 gap-3">
        <FormField
          label="信号类型"
          input-id="rule-signal"
          :error="errors.signal_type"
          :hint="
            isEdit ? '信号类型创建后不可修改' : TICKET_RULE_SIGNAL_META[signalType].description
          "
        >
          <select id="rule-signal" v-model="signalType" class="select" :disabled="isEdit">
            <option v-for="key in TICKET_RULE_SIGNAL_ORDER" :key="key" :value="key">
              {{ TICKET_RULE_SIGNAL_META[key].label }}
            </option>
          </select>
        </FormField>
        <FormField label="优先级" input-id="rule-priority" :error="errors.priority">
          <select id="rule-priority" v-model="priority" class="select">
            <option v-for="key in TICKET_PRIORITY_ORDER" :key="key" :value="key">
              {{ ticketPriorityMeta(key).label }}
            </option>
          </select>
        </FormField>
      </div>

      <div class="grid grid-cols-2 gap-3">
        <FormField
          label="生效范围"
          input-id="rule-scope"
          :hint="scopeSupported ? scopeMeta.description : '镜像验证是全局信号，范围固定为全局。'"
        >
          <select id="rule-scope" v-model="scopeType" class="select" :disabled="!scopeSupported">
            <option v-for="key in TICKET_RULE_SCOPE_ORDER" :key="key" :value="key">
              {{ TICKET_RULE_SCOPE_META[key].label }}
            </option>
          </select>
        </FormField>

        <FormField
          v-if="scopeType !== 'global'"
          :label="scopeType === 'area' ? '区域' : '房间'"
          input-id="rule-scope-id"
          :error="errors.scope_id"
          :hint="
            availableScopeOptions.length
              ? undefined
              : '当前账号没有区域 / 房间读取权限，可直接填写 UUID'
          "
        >
          <select
            v-if="availableScopeOptions.length"
            id="rule-scope-id"
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
            id="rule-scope-id"
            v-model="scopeId"
            class="input font-mono"
            placeholder="scope UUID"
          />
        </FormField>
      </div>

      <fieldset class="rounded-md border border-line px-4 py-3">
        <legend class="px-1 text-[11.5px] font-medium text-ink-soft">阈值</legend>
        <div class="grid grid-cols-2 gap-3">
          <FormField
            v-for="field in fields"
            :key="field.key"
            :label="field.label"
            :input-id="`rule-threshold-${field.key}`"
            :error="errors.thresholds?.[field.key]"
            :hint="field.hint"
          >
            <div class="flex items-center gap-2">
              <input
                :id="`rule-threshold-${field.key}`"
                v-model="thresholds[field.key]"
                class="input"
                type="number"
                :min="field.min"
                :max="field.max"
                step="1"
              />
              <span class="shrink-0 text-[11px] text-faint">{{ field.unit }}</span>
            </div>
          </FormField>
        </div>
        <p class="mt-2 text-[10.5px] text-faint">
          允许范围：
          <span v-for="field in fields" :key="field.key" class="mr-2 font-mono">
            {{ field.key }} {{ field.min }}–{{ field.max }}
          </span>
        </p>
      </fieldset>

      <div class="flex items-center justify-between rounded-md border border-line px-4 py-3">
        <div>
          <p class="text-[11.5px] font-medium text-ink-soft">启用规则</p>
          <p class="mt-0.5 text-[10.5px] text-faint">
            停用后该信号不再自动建单，已有工单不受影响。
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
          {{ isEdit ? '保存规则' : '创建规则' }}
        </AppButton>
      </div>
    </form>
  </AppModal>
</template>
