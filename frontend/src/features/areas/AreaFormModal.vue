<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type { Area, CreateAreaRequest, UpdateAreaRequest } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'

interface FormErrors {
  code?: string
  name?: string
  timezone?: string
  metadata?: string
}

const props = defineProps<{
  open: boolean
  area: Area | null
  areas: Area[]
}>()

const emit = defineEmits<{
  close: []
  saved: [area: Area]
}>()

const toast = useToastStore()

const code = ref('')
const name = ref('')
const description = ref('')
const timezone = ref('Asia/Shanghai')
const parentId = ref('')
const webhookUrl = ref('')
const isActive = ref(true)
const metadataText = ref('')
const errors = ref<FormErrors>({})
const formError = ref('')
const submitting = ref(false)

const isEdit = computed(() => props.area !== null)
const modalTitle = computed(() => (isEdit.value ? '编辑区域' : '新建区域'))
const parentOptions = computed(() => props.areas.filter((item) => item.id !== props.area?.id))

function resetForm() {
  const area = props.area
  code.value = area?.code ?? ''
  name.value = area?.name ?? ''
  description.value = area?.description ?? ''
  timezone.value = area?.timezone || 'Asia/Shanghai'
  parentId.value = area?.parent_id ?? ''
  webhookUrl.value = area?.webhook_url ?? ''
  isActive.value = area?.is_active ?? true
  const metadata = area?.metadata ?? {}
  metadataText.value = Object.keys(metadata).length ? JSON.stringify(metadata, null, 2) : ''
  errors.value = {}
  formError.value = ''
}

watch(
  () => props.open,
  (open) => {
    if (open) resetForm()
  },
)

function parseMetadata(): { value: Record<string, unknown> | null; error: string } {
  const trimmed = metadataText.value.trim()
  if (!trimmed) return { value: null, error: '' }
  try {
    const parsed: unknown = JSON.parse(trimmed)
    if (parsed === null) return { value: null, error: '' }
    if (typeof parsed !== 'object' || Array.isArray(parsed)) {
      return { value: null, error: 'metadata 必须是 JSON 对象' }
    }
    return { value: parsed as Record<string, unknown>, error: '' }
  } catch {
    return { value: null, error: 'metadata 不是合法的 JSON' }
  }
}

async function submit() {
  const nextErrors: FormErrors = {}
  if (!code.value.trim()) nextErrors.code = '请输入区域编码'
  if (!name.value.trim()) nextErrors.name = '请输入区域名称'
  if (!timezone.value.trim()) nextErrors.timezone = '请输入 IANA 时区'
  const parsed = parseMetadata()
  if (parsed.error) nextErrors.metadata = parsed.error
  errors.value = nextErrors
  formError.value = ''
  if (Object.keys(nextErrors).length > 0) return

  const base = {
    code: code.value.trim(),
    name: name.value.trim(),
    description: description.value.trim(),
    timezone: timezone.value.trim(),
    parent_id: parentId.value || null,
    webhook_url: webhookUrl.value.trim() || null,
    is_active: isActive.value,
  }

  submitting.value = true
  try {
    if (props.area) {
      const payload: UpdateAreaRequest = { ...base, metadata: parsed.value }
      const saved = await api.updateArea(props.area.id, payload)
      toast.success('区域已更新', saved.name)
      emit('saved', saved)
    } else {
      const payload: CreateAreaRequest = { ...base, metadata: parsed.value ?? undefined }
      const saved = await api.createArea(payload)
      toast.success('区域已创建', saved.name)
      emit('saved', saved)
    }
  } catch (caught) {
    if (isForbidden(caught)) {
      formError.value = '缺少权限：当前账号没有区域管理权限（area:manage）。'
    } else {
      formError.value = errorMessage(caught, '保存区域失败')
    }
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal :open="open" :title="modalTitle" size="lg" @close="emit('close')">
    <form class="space-y-3.5" novalidate @submit.prevent="submit">
      <div
        v-if="formError"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] leading-5 text-rose-700"
      >
        {{ formError }}
      </div>

      <div class="grid grid-cols-2 gap-3">
        <FormField
          label="区域编码"
          input-id="area-code"
          required
          :error="errors.code"
          hint="唯一编码，用于接口与日志"
        >
          <input
            id="area-code"
            v-model="code"
            data-autofocus
            class="input font-mono"
            placeholder="building-a"
          />
        </FormField>
        <FormField label="区域名称" input-id="area-name" required :error="errors.name">
          <input id="area-name" v-model="name" class="input" placeholder="A 栋" />
        </FormField>
      </div>

      <FormField label="描述" input-id="area-description">
        <textarea
          id="area-description"
          v-model="description"
          class="textarea"
          placeholder="可选，说明区域用途"
        />
      </FormField>

      <div class="grid grid-cols-2 gap-3">
        <FormField
          label="时区"
          input-id="area-timezone"
          required
          :error="errors.timezone"
          hint="IANA 时区，变更后后端会重建该区域的排课实例"
        >
          <input
            id="area-timezone"
            v-model="timezone"
            class="input font-mono"
            placeholder="Asia/Shanghai"
          />
        </FormField>
        <FormField label="父区域" input-id="area-parent" hint="可留空表示顶级区域">
          <select id="area-parent" v-model="parentId" class="select">
            <option value="">无（顶级区域）</option>
            <option v-for="option in parentOptions" :key="option.id" :value="option.id">
              {{ option.name }} ({{ option.code }})
            </option>
          </select>
        </FormField>
      </div>

      <FormField
        label="Webhook 地址"
        input-id="area-webhook"
        hint="区域级上下线通知地址，留空表示不推送"
      >
        <input
          id="area-webhook"
          v-model="webhookUrl"
          class="input font-mono"
          placeholder="https://example.com/hooks/area"
        />
      </FormField>

      <label class="flex items-center gap-2 text-[11.5px] text-ink-soft">
        <input
          v-model="isActive"
          type="checkbox"
          class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
        />
        启用该区域
      </label>

      <FormField
        label="metadata（JSON 对象）"
        input-id="area-metadata"
        :error="errors.metadata"
        hint='可选，例如 {"building":"A","floor":3}'
      >
        <textarea
          id="area-metadata"
          v-model="metadataText"
          class="textarea font-mono"
          spellcheck="false"
          placeholder="{}"
        />
      </FormField>
    </form>

    <template #footer>
      <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
      <AppButton variant="primary" :loading="submitting" @click="submit">
        {{ isEdit ? '保存修改' : '创建区域' }}
      </AppButton>
    </template>
  </AppModal>
</template>
