<script setup lang="ts">
/**
 * 工单附件（P1 Wave 2）：列表 + 预签名直传 + 删除。
 * 上传流程：presign → PUT object storage → register → 刷新列表（契约 task-13 §C）。
 */
import { computed, ref, watch } from 'vue'

import { ApiError, api, isForbidden } from '@/api'
import type { TicketAttachment } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import IconButton from '@/components/ui/IconButton.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatRelative } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

import { attachmentSizeLabel, attachmentTypeLabel, validateAttachmentFile } from './ticketMeta'

const props = defineProps<{
  ticketId: string
}>()

const auth = useAuthStore()
const toast = useToastStore()

const attachments = ref<TicketAttachment[]>([])
const loading = ref(false)
const error = ref('')
const uploading = ref(false)
const deleting = ref(false)
const deleteTarget = ref<TicketAttachment | null>(null)
const fileInput = ref<HTMLInputElement | null>(null)

const canUpload = computed(() =>
  auth.canAny(PERMISSIONS.ticketAttachment, PERMISSIONS.ticketHandle),
)
const canDelete = computed(() => canUpload.value || auth.can(PERMISSIONS.ticketManage))
const totalSize = computed(() => attachments.value.reduce((sum, item) => sum + (item.size ?? 0), 0))

async function load() {
  if (!props.ticketId) return
  loading.value = true
  error.value = ''
  try {
    attachments.value = await api.listTicketAttachments(props.ticketId)
  } catch (caught) {
    attachments.value = []
    if (isForbidden(caught)) {
      error.value = '缺少权限：当前账号没有 ticket:read，无法查看附件。'
    } else {
      error.value = errorMessage(caught, '加载附件失败')
    }
  } finally {
    loading.value = false
  }
}

function pickFile() {
  fileInput.value?.click()
}

async function onFilePicked(event: Event) {
  const input = event.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (!file) return

  const contentType = file.type || 'application/octet-stream'
  const validation = validateAttachmentFile({
    filename: file.name,
    size: file.size,
    contentType,
  })
  if (validation) {
    toast.error('无法上传附件', validation)
    return
  }

  uploading.value = true
  try {
    // 走后端代理上传（同源，不依赖对象存储 CORS 配置）。
    await api.uploadTicketAttachment(props.ticketId, file)
    toast.success('附件已上传', file.name)
    await load()
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '需要 ticket:attachment 或 ticket:handle。')
    } else if (caught instanceof ApiError && caught.status === 503) {
      toast.error('附件服务未配置', '后端对象存储未配置，暂时无法上传附件。')
    } else {
      toast.error('上传附件失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    uploading.value = false
  }
}

async function download(attachment: TicketAttachment) {
  try {
    const blob = await api.downloadTicketAttachment(props.ticketId, attachment.id)
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = attachment.filename
    document.body.append(anchor)
    anchor.click()
    anchor.remove()
    URL.revokeObjectURL(url)
  } catch (caught) {
    if (caught instanceof ApiError && caught.status === 503) {
      toast.error('暂不可下载', '对象存储未配置，暂时无法下载附件。')
    } else {
      toast.error('下载附件失败', errorMessage(caught, '请稍后重试'))
    }
  }
}

function askDelete(attachment: TicketAttachment) {
  deleteTarget.value = attachment
}

async function confirmDelete() {
  const target = deleteTarget.value
  if (!target) return
  deleting.value = true
  try {
    await api.deleteTicketAttachment(props.ticketId, target.id)
    toast.success('附件已删除', target.filename)
    deleteTarget.value = null
    await load()
  } catch (caught) {
    if (isForbidden(caught)) {
      toast.error('缺少权限', '只能删除自己上传的附件，或需要 ticket:manage。')
    } else {
      toast.error('删除附件失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    deleting.value = false
  }
}

watch(
  () => props.ticketId,
  () => void load(),
  { immediate: true },
)
</script>

<template>
  <div class="px-5 py-4">
    <div class="flex items-start justify-between gap-3">
      <p class="text-[11px] text-faint">
        <template v-if="attachments.length">
          共 {{ attachments.length }} 个附件 · 合计 {{ attachmentSizeLabel(totalSize) }}
        </template>
        <template v-else>支持图片 / PDF / 文本 / ZIP，单个文件 ≤ 20 MiB。</template>
      </p>
      <AppButton v-if="canUpload" size="sm" icon="upload" :loading="uploading" @click="pickFile">
        上传附件
      </AppButton>
      <input ref="fileInput" type="file" class="hidden" @change="onFilePicked" />
    </div>

    <div v-if="loading" class="mt-3 space-y-2">
      <div v-for="row in 2" :key="row" class="h-9 animate-pulse rounded-md bg-canvas" />
    </div>

    <div
      v-else-if="error"
      class="mt-3 flex items-center justify-between rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <p v-else-if="!attachments.length" class="mt-3 text-[11px] text-faint">
      暂无附件。可上传截图、日志或复现材料，便于交接与回溯。
    </p>

    <ul v-else class="mt-3 flex flex-col divide-y divide-line/70">
      <li
        v-for="attachment in attachments"
        :key="attachment.id"
        class="flex items-center gap-3 py-2.5 first:pt-0 last:pb-0"
      >
        <span
          class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-line bg-canvas/60 text-[10px] font-medium text-muted"
        >
          {{ attachmentTypeLabel(attachment.content_type).slice(0, 2) }}
        </span>
        <div class="min-w-0 flex-1">
          <p class="truncate text-[11.5px] font-medium text-ink" :title="attachment.filename">
            {{ attachment.filename }}
          </p>
          <p class="mt-0.5 truncate text-[10px] text-faint">
            {{ attachmentSizeLabel(attachment.size) }} ·
            {{ attachmentTypeLabel(attachment.content_type) }}
            <template v-if="attachment.uploaded_by_name">
              · {{ attachment.uploaded_by_name }}</template
            >
            · {{ formatRelative(attachment.created_at) }}
          </p>
        </div>
        <div class="flex shrink-0 items-center gap-1">
          <IconButton
            icon="download"
            :label="`下载 ${attachment.filename}`"
            size="sm"
            @click="download(attachment)"
          />
          <IconButton
            v-if="canDelete"
            icon="trash"
            :label="`删除 ${attachment.filename}`"
            size="sm"
            @click="askDelete(attachment)"
          />
        </div>
      </li>
    </ul>

    <ConfirmDialog
      :open="deleteTarget !== null"
      title="删除附件"
      :description="`确认删除附件「${deleteTarget?.filename ?? ''}」吗？`"
      detail="删除后对象存储中的文件将不可再通过工单下载，且不可恢复。"
      confirm-label="删除附件"
      danger
      :loading="deleting"
      @close="deleteTarget = null"
      @confirm="confirmDelete"
    />
  </div>
</template>
