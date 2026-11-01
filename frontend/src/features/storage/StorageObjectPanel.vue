<script setup lang="ts">
import { computed, reactive, ref } from 'vue'

import { api, isForbidden, isNotFound } from '@/api'
import type {
  PresignObjectReply,
  StorageObject,
  StorageObjectReply,
  StorageObjectRequest,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import CopyText from '@/components/ui/CopyText.vue'
import FormField from '@/components/ui/FormField.vue'
import InfoList from '@/components/ui/InfoList.vue'
import PageSection from '@/components/ui/PageSection.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatBytes, formatFullDateTime, shortId } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const auth = useAuthStore()
const toast = useToastStore()

const SHA_RE = /^[0-9a-f]{64}$/
const canManage = computed(() => auth.can(PERMISSIONS.storageManage))

// ---- 按 sha256 查询对象 ----
const lookupSha = ref('')
const lookupLoading = ref(false)
const lookupError = ref('')
const lookupReply = ref<StorageObjectReply | null>(null)

const lookupItems = computed(() => {
  const object = lookupReply.value?.object
  if (!object) return []
  return [
    { label: 'bucket', value: object.bucket, mono: true },
    { label: 'object_key', value: object.object_key, mono: true },
    { label: 'sha256', value: object.sha256, mono: true },
    { label: '大小', value: formatBytes(object.size) },
    { label: 'content_type', value: object.content_type, mono: true },
    { label: '创建时间', value: formatFullDateTime(object.created_at) },
  ]
})

async function lookup() {
  if (lookupLoading.value) return

  lookupError.value = ''
  const sha = lookupSha.value.trim()
  if (!SHA_RE.test(sha)) {
    lookupError.value = 'sha256 必须是 64 位小写十六进制字符串'
    lookupReply.value = null
    return
  }

  lookupLoading.value = true
  try {
    lookupReply.value = await api.getStorageObject(sha)
  } catch (caught) {
    lookupReply.value = null
    if (isNotFound(caught)) {
      lookupError.value = '存储对象不存在：该 sha256 尚未在对象表中登记。'
    } else if (isForbidden(caught)) {
      lookupError.value = '缺少 storage:read 权限，无法查询存储对象。'
    } else {
      lookupError.value = errorMessage(caught, '查询存储对象失败')
    }
  } finally {
    lookupLoading.value = false
  }
}

// ---- 预签名上传 / 登记对象（共用请求字段） ----
interface ObjectForm {
  sha256: string
  size: number
  contentType: string
}

const presignForm = reactive<ObjectForm>({ sha256: '', size: 0, contentType: '' })
const presignLoading = ref(false)
const presignError = ref('')
const presignReply = ref<PresignObjectReply | null>(null)

const registerForm = reactive<ObjectForm>({ sha256: '', size: 0, contentType: '' })
const registerLoading = ref(false)
const registerError = ref('')
const registerResult = ref<StorageObject | null>(null)

function validateForm(form: ObjectForm): string {
  if (!SHA_RE.test(form.sha256.trim())) {
    return 'sha256 必须是 64 位小写十六进制字符串'
  }
  if (!Number.isInteger(form.size) || form.size <= 0) {
    return 'size 必须是正整数（字节）'
  }
  return ''
}

function toRequest(form: ObjectForm): StorageObjectRequest {
  const contentType = form.contentType.trim()
  return {
    sha256: form.sha256.trim(),
    size: form.size,
    content_type: contentType || undefined,
  }
}

async function presign() {
  if (presignLoading.value) return

  const message = validateForm(presignForm)
  if (message) {
    presignError.value = message
    return
  }

  presignError.value = ''
  presignLoading.value = true
  try {
    presignReply.value = await api.presignObject(toRequest(presignForm))
  } catch (caught) {
    presignReply.value = null
    presignError.value = isForbidden(caught)
      ? '缺少 storage:manage 权限，无法生成预签名上传地址。'
      : errorMessage(caught, '生成预签名上传地址失败')
  } finally {
    presignLoading.value = false
  }
}

async function register() {
  if (registerLoading.value) return

  const message = validateForm(registerForm)
  if (message) {
    registerError.value = message
    return
  }

  registerError.value = ''
  registerLoading.value = true
  try {
    registerResult.value = await api.registerObject(toRequest(registerForm))
    toast.success('对象已登记', formatBytes(registerResult.value.size))
  } catch (caught) {
    registerResult.value = null
    registerError.value = isForbidden(caught)
      ? '缺少 storage:manage 权限，无法登记对象。'
      : errorMessage(caught, '登记对象失败')
    toast.error('登记对象失败', registerError.value)
  } finally {
    registerLoading.value = false
  }
}
</script>

<template>
  <div class="flex flex-col gap-4">
    <PageSection
      title="查询对象"
      description="按 sha256 查询已登记的 S3 对象，并获取短期预签名下载地址"
    >
      <div class="space-y-3 px-5 py-4">
        <form class="flex items-start gap-3" novalidate @submit.prevent="lookup">
          <FormField label="sha256" input-id="lookup-sha256" class="min-w-0 flex-1">
            <input
              id="lookup-sha256"
              v-model="lookupSha"
              class="input font-mono"
              type="text"
              placeholder="64 位小写十六进制，例如 402635e8…"
              spellcheck="false"
            />
          </FormField>
          <div class="pt-[26px]">
            <AppButton type="submit" variant="primary" icon="search" :loading="lookupLoading">
              查询
            </AppButton>
          </div>
        </form>

        <p
          v-if="lookupError"
          role="alert"
          class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] leading-5 text-rose-700"
        >
          {{ lookupError }}
        </p>

        <div v-if="lookupReply" class="rounded-md border border-line bg-canvas/50 px-4 py-3">
          <InfoList :items="lookupItems" :columns="2">
            <template #object_key>
              <CopyText
                :value="lookupReply.object.object_key"
                :display="lookupReply.object.object_key"
              />
            </template>
            <template #sha256>
              <CopyText
                :value="lookupReply.object.sha256"
                :display="shortId(lookupReply.object.sha256, 20)"
              />
            </template>
          </InfoList>
          <div class="mt-3 border-t border-line pt-3">
            <p class="text-[10.5px] text-faint">预签名下载地址</p>
            <div class="mt-0.5">
              <CopyText
                :value="lookupReply.download_url"
                :display="shortId(lookupReply.download_url, 56)"
                label="下载地址"
              />
            </div>
            <p class="mt-1.5 text-[10.5px] text-faint">
              过期时间 {{ formatFullDateTime(lookupReply.expires_at) }}，过期后重新查询即可。
            </p>
          </div>
        </div>
      </div>
    </PageSection>

    <div class="grid grid-cols-2 items-start gap-4">
      <PageSection title="预签名上传" description="为 objects/{sha256} 生成短期预签名 PUT 地址">
        <div v-if="!canManage" class="px-5 py-6 text-[11.5px] text-muted">
          当前账号没有 storage:manage 权限，只能查询对象。
        </div>
        <form v-else class="space-y-3 px-5 py-4" novalidate @submit.prevent="presign">
          <FormField
            label="sha256"
            input-id="presign-sha256"
            required
            hint="上传完成后内容必须与声明的 sha256 / size 一致"
          >
            <input
              id="presign-sha256"
              v-model="presignForm.sha256"
              class="input font-mono"
              type="text"
              placeholder="64 位小写十六进制"
              spellcheck="false"
            />
          </FormField>

          <div class="grid grid-cols-2 gap-3">
            <FormField label="size（字节）" input-id="presign-size" required>
              <input
                id="presign-size"
                v-model.number="presignForm.size"
                class="input"
                type="number"
                min="1"
                placeholder="例如 16777216"
              />
            </FormField>
            <FormField label="content_type" input-id="presign-content-type" hint="可留空">
              <input
                id="presign-content-type"
                v-model="presignForm.contentType"
                class="input"
                type="text"
                placeholder="application/octet-stream"
              />
            </FormField>
          </div>

          <p
            v-if="presignError"
            role="alert"
            class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] leading-5 text-rose-700"
          >
            {{ presignError }}
          </p>

          <AppButton variant="primary" icon="upload" :loading="presignLoading" @click="presign">
            生成上传地址
          </AppButton>

          <div v-if="presignReply" class="rounded-md border border-line bg-canvas/50 px-3 py-2.5">
            <dl class="space-y-2">
              <div>
                <dt class="text-[10.5px] text-faint">上传地址（PUT）</dt>
                <dd class="mt-0.5">
                  <CopyText
                    :value="presignReply.upload_url"
                    :display="shortId(presignReply.upload_url, 48)"
                    label="上传地址"
                  />
                </dd>
              </div>
              <div>
                <dt class="text-[10.5px] text-faint">object_key</dt>
                <dd class="mt-0.5">
                  <CopyText :value="presignReply.object_key" :display="presignReply.object_key" />
                </dd>
              </div>
              <div class="flex items-center justify-between gap-2">
                <dt class="text-[10.5px] text-faint">bucket</dt>
                <dd class="font-mono text-[11px] text-ink-soft">{{ presignReply.bucket }}</dd>
              </div>
              <div class="flex items-center justify-between gap-2">
                <dt class="text-[10.5px] text-faint">过期时间</dt>
                <dd class="text-[11px] text-ink-soft">
                  {{ formatFullDateTime(presignReply.expires_at) }}
                </dd>
              </div>
            </dl>
            <p
              class="mt-2 rounded border border-amber-200 bg-amber-50 px-2.5 py-1.5 text-[10.5px] leading-5 text-amber-700"
            >
              浏览器直连 S3 上传可能被 CORS 拦截，需要 bucket 配置允许来源与 PUT
              方法；节点侧上传不受浏览器限制。
            </p>
          </div>
        </form>
      </PageSection>

      <PageSection
        title="登记对象"
        description="上传完成后登记对象；后端会读回 S3 并重算 sha256 / size"
      >
        <div v-if="!canManage" class="px-5 py-6 text-[11.5px] text-muted">
          当前账号没有 storage:manage 权限，只能查询对象。
        </div>
        <form v-else class="space-y-3 px-5 py-4" novalidate @submit.prevent="register">
          <FormField
            label="sha256"
            input-id="register-sha256"
            required
            hint="服务端会从 S3 读回并校验，不一致返回 400"
          >
            <input
              id="register-sha256"
              v-model="registerForm.sha256"
              class="input font-mono"
              type="text"
              placeholder="64 位小写十六进制"
              spellcheck="false"
            />
          </FormField>

          <div class="grid grid-cols-2 gap-3">
            <FormField label="size（字节）" input-id="register-size" required>
              <input
                id="register-size"
                v-model.number="registerForm.size"
                class="input"
                type="number"
                min="1"
                placeholder="例如 16777216"
              />
            </FormField>
            <FormField label="content_type" input-id="register-content-type" hint="可留空">
              <input
                id="register-content-type"
                v-model="registerForm.contentType"
                class="input"
                type="text"
                placeholder="application/octet-stream"
              />
            </FormField>
          </div>

          <p
            v-if="registerError"
            role="alert"
            class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] leading-5 text-rose-700"
          >
            {{ registerError }}
          </p>

          <AppButton variant="primary" icon="check" :loading="registerLoading" @click="register">
            登记对象
          </AppButton>

          <div v-if="registerResult" class="rounded-md border border-line bg-canvas/50 px-3 py-2.5">
            <p class="text-[10.5px] text-faint">已登记对象</p>
            <InfoList
              class="mt-1.5"
              :items="[
                { label: 'object_key', value: registerResult.object_key, mono: true },
                { label: 'bucket', value: registerResult.bucket, mono: true },
                { label: 'sha256', value: registerResult.sha256, mono: true },
                { label: '大小', value: formatBytes(registerResult.size) },
                { label: 'content_type', value: registerResult.content_type, mono: true },
                { label: '创建时间', value: formatFullDateTime(registerResult.created_at) },
              ]"
              :columns="2"
            >
              <template #object_key>
                <CopyText :value="registerResult.object_key" :display="registerResult.object_key" />
              </template>
              <template #sha256>
                <CopyText
                  :value="registerResult.sha256"
                  :display="shortId(registerResult.sha256, 20)"
                />
              </template>
            </InfoList>
          </div>
        </form>
      </PageSection>
    </div>
  </div>
</template>
