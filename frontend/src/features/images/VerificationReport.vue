<script setup lang="ts">
import { computed } from 'vue'

import type { ImageVerification } from '@/api/schema'
import CopyText from '@/components/ui/CopyText.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { formatFullDateTime, shortId } from '@/utils/format'

const props = defineProps<{
  verification: ImageVerification | null
}>()

const checks = computed(() => props.verification?.report?.checks ?? [])
const errors = computed(() => props.verification?.report?.errors ?? [])
const passed = computed(() => checks.value.filter((check) => check.ok).length)
</script>

<template>
  <PageSection
    title="验证报告"
    :description="
      verification ? `最近一次同步验证：${checks.length} 项检查，${passed} 项通过` : '尚无验证记录'
    "
  >
    <div v-if="verification" class="px-5 py-4">
      <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
        <StateBadge
          :label="verification.ok ? '验证通过' : '验证未通过'"
          :tone="verification.ok ? 'emerald' : 'rose'"
        />
        <span class="text-[11px] text-muted">
          验证时间 {{ formatFullDateTime(verification.verified_at) }}
        </span>
        <span class="text-[11px] text-muted"> 执行人 {{ shortId(verification.verified_by) }} </span>
        <span class="flex items-center gap-1 text-[11px] text-muted">
          manifest_digest
          <CopyText
            :value="verification.report?.manifest_digest"
            :display="shortId(verification.report?.manifest_digest, 16)"
          />
        </span>
      </div>

      <div
        v-if="checks.length"
        class="mt-3 max-h-[340px] overflow-auto rounded-md border border-line"
      >
        <table class="w-full border-collapse text-left">
          <thead>
            <tr class="sticky top-0 z-10 border-b border-line bg-canvas">
              <th class="table-th pl-4">检查项</th>
              <th class="table-th w-20">结果</th>
              <th class="table-th pr-4">详情</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="(check, index) in checks"
              :key="`${check.name ?? 'check'}-${index}`"
              class="border-b border-line/70 last:border-b-0"
            >
              <td class="table-td pl-4 font-mono text-[10.5px]">{{ check.name ?? '—' }}</td>
              <td class="table-td">
                <span
                  class="text-[11px] font-medium"
                  :class="check.ok ? 'text-emerald-700' : 'text-rose-600'"
                >
                  {{ check.ok ? '通过' : '失败' }}
                </span>
              </td>
              <td class="table-td pr-4 text-[10.5px] leading-5 text-muted">
                {{ check.detail ?? '—' }}
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div
        v-if="errors.length"
        class="mt-3 rounded-md border border-rose-200 bg-rose-50 px-3 py-2.5"
        role="alert"
      >
        <p class="text-[11px] font-medium text-rose-700">错误</p>
        <ul class="mt-1 list-disc space-y-0.5 pl-4 text-[11px] leading-5 text-rose-700">
          <li v-for="(error, index) in errors" :key="index">{{ error }}</li>
        </ul>
      </div>
    </div>

    <EmptyState
      v-else
      title="尚无验证记录"
      description="镜像至少需要 manifest_ready 状态才能执行同步验证；验证结果会写入报告。"
      icon="shield"
    />
  </PageSection>
</template>
