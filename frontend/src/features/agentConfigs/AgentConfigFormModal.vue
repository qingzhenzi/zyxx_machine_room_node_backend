<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { api } from '@/api'
import type {
  AgentConfig,
  AgentPolicy,
  AgentPolicyPatch,
  Area,
  Room,
  UpdateAgentConfigReply,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
import {
  MAX_CACHE_BYTES,
  POLICY_DEFAULTS,
  SCOPE_LABELS,
  SCOPE_ORDER,
  cacheGiB,
} from '@/features/agentConfigs/policy'
import { errorMessage } from '@/utils/errors'

type PolicyKey = keyof AgentPolicy
type ScopeType = 'global' | 'area' | 'room'

const props = defineProps<{
  open: boolean
  config: AgentConfig | null
  areas: Area[]
  rooms: Room[]
}>()

const emit = defineEmits<{
  close: []
  saved: [reply: UpdateAgentConfigReply]
}>()

const scopeType = ref<ScopeType>('global')
const scopeId = ref('')
const override = ref<Record<PolicyKey, boolean>>({
  lxc_auto_start: true,
  poll_interval_secs: true,
  download_concurrency: true,
  cache_max_bytes: true,
})
const lxcAutoStart = ref(POLICY_DEFAULTS.lxc_auto_start)
const pollInterval = ref(String(POLICY_DEFAULTS.poll_interval_secs))
const downloadConcurrency = ref(String(POLICY_DEFAULTS.download_concurrency))
const cacheBytes = ref(String(POLICY_DEFAULTS.cache_max_bytes))
const submitting = ref(false)
const error = ref('')

const editing = computed(() => props.config !== null)
const scopeOptions = computed(() =>
  SCOPE_ORDER.map((type) => ({ value: type, label: `${SCOPE_LABELS[type]}作用域` })),
)
const cacheHint = computed(() => {
  const bytes = Number(cacheBytes.value)
  if (!Number.isFinite(bytes) || bytes < 0) return '默认 10 GiB'
  return `约 ${cacheGiB(bytes)} · 默认 10 GiB（10737418240 字节）`
})

watch(
  () => props.open,
  (open) => {
    if (!open) return
    const config = props.config
    const patch = config?.policy
    const isEdit = config !== null
    scopeType.value = config?.scope_type ?? 'global'
    scopeId.value = config?.scope_id ?? ''
    override.value = {
      lxc_auto_start: isEdit ? patch?.lxc_auto_start !== undefined : true,
      poll_interval_secs: isEdit ? patch?.poll_interval_secs !== undefined : true,
      download_concurrency: isEdit ? patch?.download_concurrency !== undefined : true,
      cache_max_bytes: isEdit ? patch?.cache_max_bytes !== undefined : true,
    }
    lxcAutoStart.value = patch?.lxc_auto_start ?? POLICY_DEFAULTS.lxc_auto_start
    pollInterval.value = String(patch?.poll_interval_secs ?? POLICY_DEFAULTS.poll_interval_secs)
    downloadConcurrency.value = String(
      patch?.download_concurrency ?? POLICY_DEFAULTS.download_concurrency,
    )
    cacheBytes.value = String(patch?.cache_max_bytes ?? POLICY_DEFAULTS.cache_max_bytes)
    submitting.value = false
    error.value = ''
  },
)

watch(scopeType, (next) => {
  if (next === 'global') scopeId.value = ''
})

function buildPolicy(): AgentPolicyPatch | null {
  const patch: AgentPolicyPatch = {}
  if (override.value.lxc_auto_start) patch.lxc_auto_start = lxcAutoStart.value

  if (override.value.poll_interval_secs) {
    const value = Number(pollInterval.value)
    if (!Number.isInteger(value) || value < 1 || value > 60) {
      error.value = '轮询间隔必须是 1-60 的整数（秒）'
      return null
    }
    patch.poll_interval_secs = value
  }

  if (override.value.download_concurrency) {
    const value = Number(downloadConcurrency.value)
    if (!Number.isInteger(value) || value < 1 || value > 32) {
      error.value = '下载并发必须是 1-32 的整数'
      return null
    }
    patch.download_concurrency = value
  }

  if (override.value.cache_max_bytes) {
    const value = Number(cacheBytes.value)
    if (!Number.isInteger(value) || value < 0 || value > MAX_CACHE_BYTES) {
      error.value = '缓存上限必须是 0 到 1 TiB（1099511627776 字节）之间的整数'
      return null
    }
    patch.cache_max_bytes = value
  }

  return patch
}

async function submit() {
  error.value = ''
  if (scopeType.value !== 'global' && !scopeId.value) {
    error.value = '区域 / 房间作用域必须选择对应目标'
    return
  }
  const policy = buildPolicy()
  if (!policy) return

  submitting.value = true
  try {
    const reply = await api.updateAgentConfig({
      scope_type: scopeType.value,
      scope_id: scopeType.value === 'global' ? null : scopeId.value,
      policy,
    })
    emit('saved', reply)
  } catch (caught) {
    error.value = errorMessage(caught, '保存节点配置失败')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="open"
    :title="editing ? '编辑作用域配置' : '新建作用域配置'"
    description="写入策略 patch 后，后端会向该作用域下已绑定设备槽位的房间二次下发 config.update。"
    size="lg"
    @close="emit('close')"
  >
    <div
      v-if="error"
      role="alert"
      class="mb-4 flex items-start gap-2 rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] leading-5 text-rose-700"
    >
      <AppIcon name="alert" :size="13" class="mt-0.5 shrink-0" />
      <span>{{ error }}</span>
    </div>

    <div class="space-y-4">
      <div class="grid grid-cols-2 gap-4">
        <FormField label="作用域类型" input-id="config-scope-type" required>
          <select id="config-scope-type" v-model="scopeType" class="select" :disabled="editing">
            <option v-for="option in scopeOptions" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
          <p v-if="editing" class="mt-1.5 text-[10.5px] text-faint">
            作用域创建后不可修改；如需其他作用域请新建配置。
          </p>
        </FormField>

        <FormField
          v-if="scopeType === 'area'"
          label="目标区域"
          input-id="config-scope-area"
          required
        >
          <select id="config-scope-area" v-model="scopeId" class="select" :disabled="editing">
            <option value="" disabled>请选择区域</option>
            <option v-for="area in areas" :key="area.id" :value="area.id">
              {{ area.name }}（{{ area.code }}）
            </option>
          </select>
        </FormField>

        <FormField
          v-else-if="scopeType === 'room'"
          label="目标房间"
          input-id="config-scope-room"
          required
        >
          <select id="config-scope-room" v-model="scopeId" class="select" :disabled="editing">
            <option value="" disabled>请选择房间</option>
            <option v-for="room in rooms" :key="room.id" :value="room.id">
              {{ room.name }}（{{ room.area_name }}）
            </option>
          </select>
        </FormField>

        <div v-else class="flex items-end pb-1">
          <p class="text-[10.5px] leading-5 text-faint">
            全局作用域不指定 scope_id，策略对所有区域与房间生效。
          </p>
        </div>
      </div>

      <div class="rounded-md border border-line">
        <div class="border-b border-line bg-canvas/40 px-3 py-2">
          <p class="text-[11.5px] font-medium text-ink-soft">策略 patch</p>
          <p class="mt-0.5 text-[10.5px] text-faint">
            取消「覆盖」表示该字段继承上层配置；全部取消即为空 patch。
          </p>
        </div>

        <div class="divide-y divide-line/70">
          <div class="flex items-center justify-between gap-4 px-3 py-2.5">
            <div class="min-w-0">
              <p class="text-[11.5px] text-ink">LXC 自动启动</p>
              <p class="text-[10.5px] text-faint">设备上线后自动拉起已分配的 LXC 容器；默认关闭</p>
            </div>
            <div class="flex shrink-0 items-center gap-3">
              <label class="flex items-center gap-1.5 text-[10.5px] text-muted">
                <input
                  v-model="override.lxc_auto_start"
                  type="checkbox"
                  class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
                />
                覆盖
              </label>
              <ToggleSwitch v-model="lxcAutoStart" :disabled="!override.lxc_auto_start" />
            </div>
          </div>

          <div class="flex items-center justify-between gap-4 px-3 py-2.5">
            <div class="min-w-0">
              <p class="text-[11.5px] text-ink">轮询间隔</p>
              <p class="text-[10.5px] text-faint">1-60 秒，默认 2 秒</p>
            </div>
            <div class="flex shrink-0 items-center gap-3">
              <label class="flex items-center gap-1.5 text-[10.5px] text-muted">
                <input
                  v-model="override.poll_interval_secs"
                  type="checkbox"
                  class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
                />
                覆盖
              </label>
              <div class="flex items-center gap-1.5">
                <input
                  v-model="pollInterval"
                  type="number"
                  min="1"
                  max="60"
                  step="1"
                  :disabled="!override.poll_interval_secs"
                  class="input h-8 w-[88px] text-right"
                />
                <span class="text-[10.5px] text-muted">秒</span>
              </div>
            </div>
          </div>

          <div class="flex items-center justify-between gap-4 px-3 py-2.5">
            <div class="min-w-0">
              <p class="text-[11.5px] text-ink">下载并发</p>
              <p class="text-[10.5px] text-faint">1-32，默认 4</p>
            </div>
            <div class="flex shrink-0 items-center gap-3">
              <label class="flex items-center gap-1.5 text-[10.5px] text-muted">
                <input
                  v-model="override.download_concurrency"
                  type="checkbox"
                  class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
                />
                覆盖
              </label>
              <div class="flex items-center gap-1.5">
                <input
                  v-model="downloadConcurrency"
                  type="number"
                  min="1"
                  max="32"
                  step="1"
                  :disabled="!override.download_concurrency"
                  class="input h-8 w-[88px] text-right"
                />
                <span class="text-[10.5px] text-muted">个</span>
              </div>
            </div>
          </div>

          <div class="flex items-start justify-between gap-4 px-3 py-2.5">
            <div class="min-w-0">
              <p class="text-[11.5px] text-ink">缓存上限</p>
              <p class="text-[10.5px] text-faint">字节数；范围 0-1 TiB，{{ cacheHint }}</p>
            </div>
            <div class="flex shrink-0 items-center gap-3 pt-0.5">
              <label class="flex items-center gap-1.5 text-[10.5px] text-muted">
                <input
                  v-model="override.cache_max_bytes"
                  type="checkbox"
                  class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
                />
                覆盖
              </label>
              <input
                v-model="cacheBytes"
                type="number"
                min="0"
                :max="MAX_CACHE_BYTES"
                step="1"
                :disabled="!override.cache_max_bytes"
                class="input h-8 w-[148px] text-right font-mono text-[11px]"
              />
            </div>
          </div>
        </div>
      </div>

      <p
        class="rounded-md border border-line bg-canvas/60 px-3 py-2 text-[10.5px] leading-5 text-muted"
      >
        默认策略：LXC 不自动启动 · 轮询 2 秒 · 下载并发 4 · 缓存上限 10 GiB。 保存后返回
        `pushed_rooms` 表示成功下发到设备槽位的房间数。
      </p>
    </div>

    <template #footer>
      <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
      <AppButton variant="primary" :loading="submitting" @click="submit">保存并下发</AppButton>
    </template>
  </AppModal>
</template>
