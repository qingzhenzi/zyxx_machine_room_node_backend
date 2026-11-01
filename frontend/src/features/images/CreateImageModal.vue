<script setup lang="ts">
import { reactive, ref, watch } from 'vue'

import { ApiError, api } from '@/api'
import type { Compression, CreateImageRequest, ImageArtifact, ImageKind } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import {
  COMPRESSION_LABELS,
  DEFAULT_FORMAT_BY_KIND,
  IMAGE_FORMAT_OPTIONS,
  IMAGE_KIND_OPTIONS,
} from './labels'

const props = defineProps<{ open: boolean }>()

const emit = defineEmits<{
  close: []
  created: [artifact: ImageArtifact]
}>()

const toast = useToastStore()

const form = reactive({
  name: '',
  version: '1',
  platform: 'linux/amd64',
  kind: 'lxc_rootfs' as ImageKind,
  format: 'tar' as CreateImageRequest['format'],
  compression: 'none' as Compression,
})

const submitting = ref(false)
const error = ref('')

const NAME_RE = /^[a-z0-9][a-z0-9._-]{0,63}$/
const VERSION_RE = /^[A-Za-z0-9][A-Za-z0-9._+-]{0,63}$/
const PLATFORM_RE = /^[A-Za-z0-9_.-]{1,32}\/[A-Za-z0-9_.-]{1,32}$/

function reset() {
  form.name = ''
  form.version = '1'
  form.platform = 'linux/amd64'
  form.kind = 'lxc_rootfs'
  form.format = 'tar'
  form.compression = 'none'
  error.value = ''
}

watch(
  () => props.open,
  (open) => {
    if (open) reset()
  },
)

watch(
  () => form.kind,
  (kind) => {
    form.format = DEFAULT_FORMAT_BY_KIND[kind]
  },
)

function validate(): string {
  if (!NAME_RE.test(form.name.trim())) {
    return '名称需为 1–64 位小写字母、数字或 - _ .，且以小写字母或数字开头'
  }
  if (!VERSION_RE.test(form.version.trim())) {
    return '版本需为 1–64 位字母、数字或 . - _ +'
  }
  if (!PLATFORM_RE.test(form.platform.trim())) {
    return '平台格式应为 系统/架构，例如 linux/amd64'
  }
  return ''
}

async function submit() {
  if (submitting.value) return

  const message = validate()
  if (message) {
    error.value = message
    return
  }

  error.value = ''
  submitting.value = true
  try {
    const input: CreateImageRequest = {
      name: form.name.trim(),
      version: form.version.trim(),
      platform: form.platform.trim(),
      kind: form.kind,
      format: form.format,
      compression: form.compression,
    }
    const artifact = await api.createImage(input)
    toast.success('镜像已创建', `${artifact.name}:${artifact.version}（${artifact.status}）`)
    emit('created', artifact)
    emit('close')
  } catch (caught) {
    if (caught instanceof ApiError && caught.status === 409) {
      error.value = `镜像已存在：名称 + 版本 + 平台 必须唯一。${caught.message}`
    } else {
      error.value = errorMessage(caught, '创建镜像失败')
    }
    toast.error('创建镜像失败', error.value)
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="open"
    title="新建镜像"
    description="创建后状态为 uploaded，随后提交 manifest 并同步验证。"
    size="md"
    @close="emit('close')"
  >
    <form class="space-y-3.5" novalidate @submit.prevent="submit">
      <div class="grid grid-cols-2 gap-3">
        <FormField label="名称" input-id="image-name" required hint="小写字母/数字/-_.">
          <input
            id="image-name"
            v-model="form.name"
            data-autofocus
            class="input"
            type="text"
            placeholder="debian12-lxc"
          />
        </FormField>
        <FormField label="版本" input-id="image-version" required hint="例如 1、2024.10">
          <input
            id="image-version"
            v-model="form.version"
            class="input"
            type="text"
            placeholder="1"
          />
        </FormField>
      </div>

      <FormField label="平台" input-id="image-platform" required hint="系统/架构，例如 linux/amd64">
        <input
          id="image-platform"
          v-model="form.platform"
          class="input"
          type="text"
          placeholder="linux/amd64"
        />
      </FormField>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="类型" input-id="image-kind" required>
          <select id="image-kind" v-model="form.kind" class="select">
            <option v-for="option in IMAGE_KIND_OPTIONS" :key="option.value" :value="option.value">
              {{ option.label }}
            </option>
          </select>
        </FormField>
        <FormField label="格式" input-id="image-format" required>
          <select id="image-format" v-model="form.format" class="select">
            <option
              v-for="option in IMAGE_FORMAT_OPTIONS"
              :key="option.value"
              :value="option.value"
            >
              {{ option.label }}
            </option>
          </select>
        </FormField>
      </div>

      <FormField label="压缩" input-id="image-compression" hint="当前后端仅支持无压缩">
        <select id="image-compression" v-model="form.compression" class="select">
          <option v-for="(label, value) in COMPRESSION_LABELS" :key="value" :value="value">
            {{ label }}
          </option>
        </select>
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
      <AppButton variant="primary" :loading="submitting" @click="submit">创建镜像</AppButton>
    </template>
  </AppModal>
</template>
