<script setup lang="ts">
import { computed, ref } from 'vue'

import { api } from '@/api'
import type { ArtifactStatus, ImageArtifact, ImageChannel } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import FormField from '@/components/ui/FormField.vue'
import PageSection from '@/components/ui/PageSection.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { IMAGE_CHANNEL_OPTIONS, artifactStatusMeta } from './labels'

const props = defineProps<{
  imageId: string
  imageName: string
  status: ArtifactStatus
}>()

const emit = defineEmits<{
  published: [artifact: ImageArtifact]
}>()

const toast = useToastStore()
const channel = ref<ImageChannel>('dev')
const confirmOpen = ref(false)
const publishing = ref(false)
const error = ref('')

const publishable = computed(() => props.status === 'verified')
const statusMeta = computed(() => artifactStatusMeta(props.status))

async function publish() {
  if (publishing.value) return

  publishing.value = true
  error.value = ''
  try {
    const artifact = await api.publishImage(props.imageId, { channel: channel.value })
    confirmOpen.value = false
    toast.success('镜像已发布', `通道 ${channel.value} · 状态 ${artifact.status}`)
    emit('published', artifact)
  } catch (caught) {
    error.value = errorMessage(caught, '发布镜像失败')
    confirmOpen.value = false
    toast.error('发布镜像失败', error.value)
  } finally {
    publishing.value = false
  }
}
</script>

<template>
  <PageSection title="发布到通道" description="发布后镜像状态变为 published，可分配到房间">
    <div class="space-y-3 px-5 py-4">
      <FormField label="目标通道" input-id="publish-channel">
        <select id="publish-channel" v-model="channel" class="select" :disabled="!publishable">
          <option v-for="option in IMAGE_CHANNEL_OPTIONS" :key="option.value" :value="option.value">
            {{ option.label }}
          </option>
        </select>
      </FormField>

      <AppButton
        variant="primary"
        icon="upload"
        :disabled="!publishable"
        :loading="publishing"
        @click="confirmOpen = true"
      >
        发布镜像
      </AppButton>

      <p v-if="!publishable" class="text-[11px] leading-5 text-muted">
        只有已验证（verified）的镜像可以发布，当前状态为
        {{ statusMeta.label }}。请先提交 manifest 并执行同步验证。
      </p>
      <p v-else class="text-[11px] leading-5 text-muted">
        发布只更新通道当前指针；分发到具体房间请使用下方的“房间分配”。
      </p>

      <p
        v-if="error"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] leading-5 text-rose-700"
      >
        {{ error }}
      </p>
    </div>

    <ConfirmDialog
      :open="confirmOpen"
      title="发布镜像"
      :description="`将把「${imageName}」发布到 ${channel} 通道。`"
      detail="发布会更新通道当前指针，之后即可把镜像分配到房间。该操作可以在后续版本中重新发布覆盖。"
      confirm-label="确认发布"
      :loading="publishing"
      @confirm="publish"
      @close="confirmOpen = false"
    />
  </PageSection>
</template>
