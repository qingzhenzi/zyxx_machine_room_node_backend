<script setup lang="ts">
import { ref } from 'vue'

import { api } from '@/api'
import type { ImageArtifact } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import { useToastStore } from '@/stores/toast'

import {
  canDeprecateImage,
  deleteConfirmDetail,
  deprecateConfirmDetail,
  imageDeleteErrorMeta,
  imageDeprecateErrorMeta,
} from './imageLifecycle'

const props = withDefaults(
  defineProps<{
    image: ImageArtifact
    size?: 'sm' | 'md'
  }>(),
  { size: 'md' },
)

const emit = defineEmits<{
  deprecated: [artifact: ImageArtifact]
  deleted: []
  /** 404 / 状态过期时由父级重新拉取列表或详情。 */
  refresh: []
}>()

const toast = useToastStore()
const deprecateOpen = ref(false)
const deleteOpen = ref(false)
const deprecating = ref(false)
const deleting = ref(false)

function label(): string {
  return `${props.image.name}:${props.image.version}`
}

async function deprecate() {
  if (deprecating.value || deleting.value) return
  deprecating.value = true
  try {
    const artifact = await api.deprecateImage(props.image.id)
    deprecateOpen.value = false
    toast.success('镜像已下架', `${label()} · 状态 ${artifact.status}`)
    emit('deprecated', artifact)
  } catch (caught) {
    deprecateOpen.value = false
    const meta = imageDeprecateErrorMeta(caught)
    toast.error(meta.title, meta.description)
    if (meta.shouldRefresh) emit('refresh')
  } finally {
    deprecating.value = false
  }
}

async function remove() {
  if (deprecating.value || deleting.value) return
  deleting.value = true
  try {
    await api.deleteImage(props.image.id)
    deleteOpen.value = false
    toast.success('镜像已删除', `${label()} 及其无引用的对象存储分片已清理`)
    emit('deleted')
  } catch (caught) {
    deleteOpen.value = false
    const meta = imageDeleteErrorMeta(caught)
    toast.error(meta.title, meta.description)
    if (meta.shouldRefresh) emit('refresh')
  } finally {
    deleting.value = false
  }
}
</script>

<template>
  <div class="flex flex-wrap items-center gap-1.5">
    <AppButton
      v-if="canDeprecateImage(image.status)"
      :size="size"
      icon="archive"
      title="下架镜像（published → deprecated，不删除对象存储）"
      :disabled="deprecating || deleting"
      @click="deprecateOpen = true"
    >
      下架
    </AppButton>
    <AppButton
      :size="size"
      variant="danger"
      icon="trash"
      title="删除镜像（危险操作，不可恢复）"
      :disabled="deprecating || deleting"
      @click="deleteOpen = true"
    >
      删除
    </AppButton>
  </div>

  <ConfirmDialog
    :open="deprecateOpen"
    title="下架镜像"
    :description="`确认下架「${image.name}:${image.version}」？`"
    :detail="deprecateConfirmDetail(image)"
    confirm-label="确认下架"
    :loading="deprecating"
    @confirm="deprecate"
    @close="deprecateOpen = false"
  />

  <ConfirmDialog
    :open="deleteOpen"
    title="删除镜像"
    :description="`确认永久删除「${image.name}:${image.version}」？`"
    :detail="deleteConfirmDetail(image)"
    confirm-label="确认删除"
    danger
    :loading="deleting"
    @confirm="remove"
    @close="deleteOpen = false"
  />
</template>
