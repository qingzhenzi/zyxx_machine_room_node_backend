<script setup lang="ts">
import { computed } from 'vue'

import type { ImageDetail } from '@/api/schema'
import CopyText from '@/components/ui/CopyText.vue'
import InfoList from '@/components/ui/InfoList.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { formatBytes, formatFullDateTime, formatRelative, shortId } from '@/utils/format'
import {
  artifactStatusMeta,
  channelMeta,
  compressionLabel,
  imageFormatLabel,
  imageKindLabel,
} from './labels'

const props = defineProps<{
  image: ImageDetail
}>()

const status = computed(() => artifactStatusMeta(props.image.status))

const items = computed(() => [
  { label: 'ID', value: props.image.id, mono: true },
  { label: '类型', value: imageKindLabel(props.image.kind) },
  { label: '平台', value: props.image.platform },
  {
    label: '格式 / 压缩',
    value: `${imageFormatLabel(props.image.format)} / ${compressionLabel(props.image.compression)}`,
  },
  { label: '总大小', value: formatBytes(props.image.total_size) },
  { label: 'manifest_digest', value: props.image.manifest_digest ?? '—', mono: true },
  { label: '创建时间', value: formatFullDateTime(props.image.created_at) },
  {
    label: '更新时间',
    value: `${formatRelative(props.image.updated_at)} · 创建者 ${shortId(props.image.created_by ?? undefined)}`,
  },
])
</script>

<template>
  <section class="rounded-xl border border-line bg-surface">
    <header class="flex items-start justify-between gap-4 border-b border-line px-5 py-4">
      <div class="min-w-0">
        <div class="flex flex-wrap items-center gap-2">
          <h2 class="truncate text-[16px] font-semibold tracking-[-0.01em] text-ink">
            {{ image.name }}:{{ image.version }}
          </h2>
          <StateBadge :label="status.label" :tone="status.tone" />
          <StateBadge
            v-for="channel in image.channels ?? []"
            :key="channel.id"
            :label="channelMeta(channel.channel).label"
            :tone="channelMeta(channel.channel).tone"
            :dot="false"
          />
        </div>
        <p class="mt-1 text-[11.5px] text-muted">
          {{ imageKindLabel(image.kind) }} · {{ image.platform }} ·
          {{ imageFormatLabel(image.format) }} / {{ compressionLabel(image.compression) }} ·
          {{ formatBytes(image.total_size) }}
        </p>
      </div>
    </header>

    <div class="px-5 py-4">
      <InfoList :items="items" :columns="2">
        <template #ID>
          <CopyText :value="image.id" :display="image.id" />
        </template>
        <template #manifest_digest>
          <CopyText
            :value="image.manifest_digest"
            :display="image.manifest_digest ? shortId(image.manifest_digest, 16) : '—'"
          />
        </template>
      </InfoList>
    </div>
  </section>
</template>
