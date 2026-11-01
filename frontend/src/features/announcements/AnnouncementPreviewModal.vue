<script setup lang="ts">
import { computed } from 'vue'

import type { Announcement } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { statusMeta } from '@/features/announcements/announcementStatus'
import { formatDateTime, formatFullDateTime } from '@/utils/format'

const props = defineProps<{
  open: boolean
  announcement: Announcement | null
}>()

const emit = defineEmits<{
  close: []
}>()

const meta = computed(() => statusMeta(props.announcement?.status))
</script>

<template>
  <AppModal
    :open="open"
    :title="announcement?.title || '公告预览'"
    description="正文以纯文本展示，不执行任何 HTML。"
    size="lg"
    @close="emit('close')"
  >
    <div class="mb-3 flex flex-wrap items-center gap-2 text-[10.5px] text-muted">
      <StateBadge :label="meta.label" :tone="meta.tone" />
      <span v-if="announcement?.is_pinned" class="rounded bg-accent-soft px-1.5 py-0.5 text-accent">
        置顶
      </span>
      <span>作者：{{ announcement?.author_name || '—' }}</span>
      <span>创建：{{ formatDateTime(announcement?.created_at) }}</span>
      <span>更新：{{ formatDateTime(announcement?.updated_at) }}</span>
      <span v-if="announcement?.published_at">
        发布：{{ formatFullDateTime(announcement.published_at) }}
      </span>
    </div>

    <pre
      class="max-h-[420px] overflow-auto whitespace-pre-wrap break-words rounded-md border border-line bg-canvas/60 px-4 py-3 font-sans text-[12px] leading-6 text-ink-soft"
      >{{ announcement?.content || '（无正文）' }}</pre>

    <template #footer>
      <span class="mr-auto font-mono text-[10px] text-faint">
        公告 ID：{{ announcement?.id || '—' }}
      </span>
      <AppButton @click="emit('close')">关闭</AppButton>
    </template>
  </AppModal>
</template>
