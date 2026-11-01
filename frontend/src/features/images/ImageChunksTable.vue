<script setup lang="ts">
import type { ImageChunk } from '@/api/schema'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import PageSection from '@/components/ui/PageSection.vue'
import { formatBytes, shortId } from '@/utils/format'

defineProps<{
  chunks: ImageChunk[]
}>()
</script>

<template>
  <PageSection
    title="分片"
    :description="`共 ${chunks.length} 个分片，按 offset 顺序拼接为完整镜像`"
  >
    <div v-if="chunks.length" class="overflow-x-auto">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">#</th>
            <th class="table-th">sha256</th>
            <th class="table-th">偏移</th>
            <th class="table-th">大小</th>
            <th class="table-th pr-5">对象 key</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="chunk in chunks"
            :key="chunk.index"
            class="border-b border-line/70 last:border-b-0 hover:bg-canvas/50"
          >
            <td class="table-td pl-5 font-mono text-[10.5px] text-faint">
              {{ chunk.index ?? '—' }}
            </td>
            <td class="table-td">
              <CopyText :value="chunk.sha256" :display="shortId(chunk.sha256, 16)" />
            </td>
            <td class="table-td font-mono text-[10.5px]">{{ chunk.offset ?? '—' }}</td>
            <td class="table-td text-[11px]">{{ formatBytes(chunk.size) }}</td>
            <td class="table-td pr-5">
              <CopyText
                :value="chunk.object_key"
                :display="shortId(chunk.object_key, 24)"
                :truncate="true"
              />
            </td>
          </tr>
        </tbody>
      </table>
    </div>
    <EmptyState
      v-else
      title="还没有分片"
      description="提交 manifest 后，分片记录会在这里按 index 顺序展示。"
      icon="layers"
    />
  </PageSection>
</template>
