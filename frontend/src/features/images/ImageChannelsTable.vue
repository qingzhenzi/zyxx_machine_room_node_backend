<script setup lang="ts">
import type { ImageChannelRow } from '@/api/schema'
import EmptyState from '@/components/ui/EmptyState.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { formatFullDateTime } from '@/utils/format'
import { channelMeta } from './labels'

defineProps<{
  channels: ImageChannelRow[]
}>()
</script>

<template>
  <PageSection title="发布通道" description="镜像发布后在 dev / test / prod 通道中的当前记录">
    <div v-if="channels.length" class="overflow-x-auto">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">通道</th>
            <th class="table-th">是否当前</th>
            <th class="table-th">artifact id</th>
            <th class="table-th pr-5">发布时间</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="channel in channels"
            :key="channel.id"
            class="border-b border-line/70 last:border-b-0 hover:bg-canvas/50"
          >
            <td class="table-td pl-5">
              <StateBadge
                :label="channelMeta(channel.channel).label"
                :tone="channelMeta(channel.channel).tone"
                :dot="false"
              />
            </td>
            <td class="table-td">
              <span
                class="text-[11px]"
                :class="channel.is_current ? 'text-emerald-700' : 'text-faint'"
              >
                {{ channel.is_current ? '是' : '否' }}
              </span>
            </td>
            <td class="table-td font-mono text-[10.5px]">{{ channel.artifact_id ?? '—' }}</td>
            <td class="table-td pr-5 text-[11px]">{{ formatFullDateTime(channel.created_at) }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <EmptyState
      v-else
      title="尚未发布"
      description="镜像通过验证后可以发布到 dev / test / prod 通道。"
      icon="package"
    />
  </PageSection>
</template>
