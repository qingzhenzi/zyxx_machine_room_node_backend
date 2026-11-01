<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { api } from '@/api'
import type { Announcement } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
import FormField from '@/components/ui/FormField.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
import { errorMessage } from '@/utils/errors'

const props = defineProps<{
  open: boolean
  announcement: Announcement | null
  canPublish: boolean
}>()

const emit = defineEmits<{
  close: []
  saved: [announcement: Announcement, created: boolean]
}>()

const editing = computed(() => props.announcement !== null)

const title = ref('')
const content = ref('')
const status = ref<'draft' | 'published' | 'archived'>('draft')
const isPinned = ref(false)
const submitting = ref(false)
const error = ref('')

const canSubmit = computed(
  () => !submitting.value && title.value.trim().length > 0 && content.value.trim().length > 0,
)

watch(
  () => props.open,
  (open) => {
    if (!open) return
    error.value = ''
    submitting.value = false
    title.value = props.announcement?.title ?? ''
    content.value = props.announcement?.content ?? ''
    status.value = props.announcement?.status ?? 'draft'
    isPinned.value = props.announcement?.is_pinned ?? false
  },
)

async function submit() {
  if (!canSubmit.value) return
  error.value = ''
  submitting.value = true
  try {
    const payload = {
      title: title.value.trim(),
      content: content.value,
      is_pinned: isPinned.value,
      ...(props.canPublish ? { status: status.value } : {}),
    }
    const saved = props.announcement?.id
      ? await api.updateAnnouncement(props.announcement.id, payload)
      : await api.createAnnouncement(payload)
    emit('saved', saved, !editing.value)
  } catch (caught) {
    error.value = errorMessage(caught, '保存公告失败')
  } finally {
    submitting.value = false
  }
}
</script>

<template>
  <AppModal
    :open="open"
    :title="editing ? '编辑公告' : '新建公告'"
    :description="
      editing
        ? '修改标题、正文与置顶；发布状态变更需要 announcement:publish 权限。'
        : '公告正文按纯文本保存与展示，不执行 HTML。'
    "
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
      <FormField label="标题" input-id="announcement-title" required hint="1-200 个字符">
        <input
          id="announcement-title"
          v-model="title"
          type="text"
          maxlength="200"
          placeholder="例如 系统维护通知"
          class="input"
        />
      </FormField>

      <FormField
        label="正文"
        input-id="announcement-content"
        required
        :hint="`${content.length} / 100000 个字符 · 纯文本展示（<pre>），不会执行 HTML`"
      >
        <textarea
          id="announcement-content"
          v-model="content"
          rows="10"
          maxlength="100000"
          placeholder="支持多行文本；展示时保留换行与缩进。"
          class="textarea min-h-[220px] font-mono text-[11.5px] leading-5"
        />
      </FormField>

      <div class="grid grid-cols-2 items-start gap-4">
        <FormField
          label="状态"
          input-id="announcement-status"
          :hint="
            canPublish
              ? '发布与归档会立即对公开接口生效'
              : '缺少 announcement:publish 权限，只能保存草稿或保留原状态'
          "
        >
          <select id="announcement-status" v-model="status" class="select" :disabled="!canPublish">
            <option value="draft">草稿（仅后台可见）</option>
            <option value="published">已发布（公开可见）</option>
            <option value="archived">已归档</option>
          </select>
        </FormField>

        <div class="flex items-center justify-between rounded-md border border-line px-3 py-2.5">
          <div class="min-w-0">
            <p class="text-[11.5px] font-medium text-ink-soft">置顶</p>
            <p class="mt-0.5 text-[10.5px] text-faint">置顶公告在公开列表中优先展示</p>
          </div>
          <ToggleSwitch v-model="isPinned" />
        </div>
      </div>

      <p
        class="rounded-md border border-line bg-canvas/60 px-3 py-2 text-[10.5px] leading-5 text-muted"
      >
        安全提示：正文由后端清洗，前端始终以纯文本渲染，不使用 v-html 解释 HTML。
      </p>
    </div>

    <template #footer>
      <AppButton :disabled="submitting" @click="emit('close')">取消</AppButton>
      <AppButton variant="primary" :loading="submitting" :disabled="!canSubmit" @click="submit">
        {{ editing ? '保存修改' : '保存公告' }}
      </AppButton>
    </template>
  </AppModal>
</template>
