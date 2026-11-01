<script setup lang="ts">
import { ref } from 'vue'

import { api, isForbidden } from '@/api'
import type { ManifestReply } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import CopyText from '@/components/ui/CopyText.vue'
import JsonBlock from '@/components/ui/JsonBlock.vue'
import PageSection from '@/components/ui/PageSection.vue'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, shortId } from '@/utils/format'

const props = defineProps<{
  imageId: string
}>()

const reply = ref<ManifestReply | null>(null)
const loading = ref(false)
const error = ref('')

async function load() {
  if (loading.value) return

  loading.value = true
  error.value = ''
  try {
    reply.value = await api.getManifest(props.imageId)
  } catch (caught) {
    reply.value = null
    error.value = isForbidden(caught)
      ? '缺少 image:read 权限，无法读取 manifest。'
      : errorMessage(caught, '读取 manifest 失败')
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <PageSection title="Manifest" description="读取已写入 S3 的 manifest 与预签名下载地址">
    <div class="space-y-3 px-5 py-4">
      <AppButton icon="download" :loading="loading" @click="load">
        {{ reply ? '重新读取 manifest' : '读取 manifest' }}
      </AppButton>

      <p
        v-if="error"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] leading-5 text-rose-700"
      >
        {{ error }}
      </p>

      <template v-if="reply">
        <dl class="space-y-2.5">
          <div>
            <dt class="text-[10.5px] text-faint">manifest_digest</dt>
            <dd class="mt-0.5">
              <CopyText
                :value="reply.manifest_digest"
                :display="shortId(reply.manifest_digest, 20)"
                label="manifest digest"
              />
            </dd>
          </div>
          <div>
            <dt class="text-[10.5px] text-faint">预签名下载地址</dt>
            <dd class="mt-0.5">
              <CopyText :value="reply.download_url" :display="shortId(reply.download_url, 52)" />
            </dd>
          </div>
          <div>
            <dt class="text-[10.5px] text-faint">下载地址过期时间</dt>
            <dd class="mt-0.5 text-[11.5px] text-ink-soft">
              {{ formatFullDateTime(reply.expires_at) }}
            </dd>
          </div>
        </dl>

        <JsonBlock :value="reply.manifest" label="manifest JSON" max-height="320px" />
      </template>

      <p v-else-if="!loading && !error" class="text-[11px] leading-5 text-muted">
        尚未读取。manifest 需镜像进入 manifest_ready 及之后状态才会写入 S3；下载地址有有效期。
      </p>
    </div>
  </PageSection>
</template>
