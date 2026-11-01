<script setup lang="ts">
import { ref, watch } from 'vue'

import { api } from '@/api'
import type {
  ManifestPart,
  ManifestRuntimeInput,
  SubmitManifestReply,
  SubmitManifestRequest,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { shortId } from '@/utils/format'

const props = defineProps<{
  open: boolean
  imageId: string
  imageName: string
  totalSize: number
}>()

const emit = defineEmits<{
  close: []
  submitted: [reply: SubmitManifestReply]
}>()

const toast = useToastStore()
const text = ref('')
const submitting = ref(false)
const error = ref('')

const SHA_RE = /^[0-9a-f]{64}$/

function buildDefault(totalSize: number): string {
  return JSON.stringify(
    {
      total_size: totalSize,
      parts: [{ index: 0, offset: 0, sha256: '0'.repeat(64), size: totalSize }],
      annotations: {},
    },
    null,
    2,
  )
}

watch(
  () => props.open,
  (open) => {
    if (!open) return
    error.value = ''
    text.value = buildDefault(props.totalSize)
  },
  { immediate: true },
)

function parseInput(): SubmitManifestRequest {
  let raw: unknown
  try {
    raw = JSON.parse(text.value)
  } catch {
    throw new Error('JSON 解析失败，请检查括号、引号与逗号')
  }
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    throw new Error('manifest 顶层必须是 JSON 对象')
  }

  const data = raw as Record<string, unknown>
  const totalSize = data.total_size
  if (typeof totalSize !== 'number' || !Number.isInteger(totalSize) || totalSize <= 0) {
    throw new Error('total_size 必须是正整数')
  }

  const rawParts = data.parts
  if (!Array.isArray(rawParts) || rawParts.length === 0) {
    throw new Error('parts 必须是非空数组')
  }

  const parts: ManifestPart[] = rawParts.map((item: unknown, index: number) => {
    if (typeof item !== 'object' || item === null || Array.isArray(item)) {
      throw new Error(`parts[${index}] 必须是对象`)
    }
    const part = item as Record<string, unknown>
    const partIndex = part.index
    const offset = part.offset
    const sha256 = part.sha256
    const size = part.size

    if (typeof partIndex !== 'number' || !Number.isInteger(partIndex) || partIndex < 0) {
      throw new Error(`parts[${index}].index 必须是非负整数`)
    }
    if (typeof offset !== 'number' || !Number.isInteger(offset) || offset < 0) {
      throw new Error(`parts[${index}].offset 必须是非负整数`)
    }
    if (typeof size !== 'number' || !Number.isInteger(size) || size <= 0) {
      throw new Error(`parts[${index}].size 必须是正整数`)
    }
    if (typeof sha256 !== 'string' || !SHA_RE.test(sha256)) {
      throw new Error(`parts[${index}].sha256 必须是 64 位小写十六进制字符串`)
    }

    return { index: partIndex, offset, sha256, size }
  })

  const sum = parts.reduce((total, part) => total + part.size, 0)
  if (sum !== totalSize) {
    throw new Error(`parts 的 size 之和（${sum}）必须等于 total_size（${totalSize}）`)
  }

  const input: SubmitManifestRequest = { total_size: totalSize, parts }

  if (data.runtime !== undefined) {
    if (typeof data.runtime !== 'object' || data.runtime === null || Array.isArray(data.runtime)) {
      throw new Error('runtime 必须是对象')
    }
    input.runtime = data.runtime as ManifestRuntimeInput
  }
  if (data.annotations !== undefined) {
    if (
      typeof data.annotations !== 'object' ||
      data.annotations === null ||
      Array.isArray(data.annotations)
    ) {
      throw new Error('annotations 必须是对象')
    }
    input.annotations = data.annotations as Record<string, unknown>
  }

  return input
}

async function submit() {
  if (submitting.value) return

  error.value = ''
  let input: SubmitManifestRequest
  try {
    input = parseInput()
  } catch (caught) {
    error.value = caught instanceof Error ? caught.message : 'manifest 校验失败'
    return
  }

  submitting.value = true
  try {
    const reply = await api.submitManifest(props.imageId, input)
    toast.success('manifest 已提交', `digest ${shortId(reply.manifest_digest, 16)}`)
    emit('submitted', reply)
    emit('close')
  } catch (caught) {
    error.value = errorMessage(caught, '提交 manifest 失败')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="open"
    title="提交 manifest"
    :description="`为 ${imageName || '镜像'} 提交分片 manifest；后端会校验分片对象、签名并写入 S3。`"
    size="lg"
    @close="emit('close')"
  >
    <form class="space-y-3" novalidate @submit.prevent="submit">
      <FormField
        label="manifest JSON"
        input-id="manifest-json"
        required
        hint="字段：total_size、parts[{index,offset,sha256,size}]；可选 runtime（docker/lxc/vm）与 annotations。index/offset 需从 0 连续，total_size 等于各分片 size 之和。"
      >
        <textarea
          id="manifest-json"
          v-model="text"
          data-autofocus
          class="textarea min-h-[320px] font-mono text-[11px] leading-[1.7]"
          spellcheck="false"
        />
      </FormField>

      <p
        v-if="error"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] leading-5 text-rose-700"
      >
        {{ error }}
      </p>
    </form>

    <template #footer>
      <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
      <AppButton variant="primary" :loading="submitting" @click="submit">提交 manifest</AppButton>
    </template>
  </AppModal>
</template>
