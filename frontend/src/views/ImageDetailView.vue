<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import { api, isForbidden, isNotFound } from '@/api'
import type { ImageArtifact, ImageDetail, ImageVerification, Room } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import AssignmentPanel from '@/features/images/AssignmentPanel.vue'
import ImageChannelsTable from '@/features/images/ImageChannelsTable.vue'
import ImageChunksTable from '@/features/images/ImageChunksTable.vue'
import ImageLifecycleActions from '@/features/images/ImageLifecycleActions.vue'
import ImageSummaryCard from '@/features/images/ImageSummaryCard.vue'
import ManifestPanel from '@/features/images/ManifestPanel.vue'
import PublishPanel from '@/features/images/PublishPanel.vue'
import SubmitManifestModal from '@/features/images/SubmitManifestModal.vue'
import VerificationReport from '@/features/images/VerificationReport.vue'
import { artifactStatusMeta } from '@/features/images/labels'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatRelative } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const route = useRoute()
const router = useRouter()
const auth = useAuthStore()
const toast = useToastStore()

const imageId = computed(() => String(route.params.id ?? ''))
const canManage = computed(() => auth.can(PERMISSIONS.imageManage))

const image = ref<ImageDetail | null>(null)
const loading = ref(false)
const error = ref('')
const notFound = ref(false)

const verifying = ref(false)
const verification = ref<ImageVerification | null>(null)
const verifyError = ref('')

const submitOpen = ref(false)

const rooms = ref<Room[]>([])
const roomsLoading = ref(false)
const roomsError = ref('')

const statusMeta = computed(() => artifactStatusMeta(image.value?.status))
const displayStatus = computed(() => (image.value ? statusMeta.value.label : ''))
const imageLabel = computed(() =>
  image.value ? `${image.value.name}:${image.value.version}` : '镜像',
)

async function loadDetail(silent = false) {
  if (!imageId.value) return

  if (!silent) {
    loading.value = true
    error.value = ''
    notFound.value = false
  }

  try {
    image.value = await api.getImage(imageId.value)
  } catch (caught) {
    if (!silent) {
      image.value = null
      if (isNotFound(caught)) {
        notFound.value = true
      } else {
        error.value = isForbidden(caught)
          ? '缺少 image:read 权限，无法查看该镜像。'
          : errorMessage(caught, '加载镜像详情失败')
      }
    }
  } finally {
    if (!silent) loading.value = false
  }
}

async function loadRooms() {
  if (!canManage.value) return

  roomsLoading.value = true
  roomsError.value = ''
  try {
    const data = await api.listRooms({ page_size: 100 })
    rooms.value = data.items ?? []
  } catch (caught) {
    roomsError.value = isForbidden(caught)
      ? '缺少 room:read 权限，无法加载房间下拉。'
      : errorMessage(caught, '加载房间列表失败')
  } finally {
    roomsLoading.value = false
  }
}

function init() {
  verification.value = null
  verifyError.value = ''
  submitOpen.value = false
  void loadDetail()
  void loadRooms()
}

watch(imageId, init, { immediate: true })

async function runVerify() {
  if (verifying.value || !imageId.value) return

  verifying.value = true
  verifyError.value = ''
  try {
    const reply = await api.verifyImage(imageId.value)
    verification.value = reply.verification
    image.value = image.value ? { ...image.value, ...reply.artifact } : image.value
    if (reply.verification.ok) {
      toast.success('同步验证通过', '验证报告已写入，可查看每项检查结果')
    } else {
      toast.error('同步验证未通过', '请查看验证报告中的失败项与错误信息')
    }
  } catch (caught) {
    verifyError.value = isForbidden(caught)
      ? '缺少 image:manage 权限，无法执行同步验证。'
      : errorMessage(caught, '同步验证失败')
    toast.error('同步验证失败', verifyError.value)
    await loadDetail(true)
  } finally {
    verifying.value = false
  }
}

async function onPublished(artifact: ImageArtifact) {
  image.value = image.value ? { ...image.value, ...artifact } : image.value
  await loadDetail(true)
}

/** 下架成功：用后端返回的最新 artifact 覆盖页面状态。 */
async function onDeprecated(artifact: ImageArtifact) {
  image.value = image.value ? { ...image.value, ...artifact } : image.value
  await loadDetail(true)
}

/** 删除成功：镜像已不存在，回到列表页。 */
function onDeleted() {
  void router.push({ name: 'images' })
}

async function onManifestSubmitted() {
  verification.value = null
  await loadDetail(true)
}
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader :title="image ? imageLabel : '镜像详情'" :badge="image ? displayStatus : ''">
      <template #description>
        <RouterLink :to="{ name: 'images' }" class="link inline-flex items-center gap-1">
          <AppIcon name="arrow-left" :size="12" />
          返回镜像列表
        </RouterLink>
        <span v-if="image"> · 更新于 {{ formatRelative(image.updated_at) }}</span>
      </template>
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新镜像详情"
          variant="outline"
          :loading="loading"
          @click="loadDetail()"
        />
        <template v-if="canManage && image">
          <AppButton icon="shield" :loading="verifying" @click="runVerify">同步验证</AppButton>
          <AppButton variant="primary" icon="upload" @click="submitOpen = true">
            提交 manifest
          </AppButton>
          <ImageLifecycleActions
            :image="image"
            @deprecated="onDeprecated"
            @deleted="onDeleted"
            @refresh="loadDetail()"
          />
        </template>
      </template>
    </PageHeader>

    <div v-if="loading" class="mt-4 grid grid-cols-12 items-start gap-4">
      <div class="col-span-8 flex flex-col gap-4">
        <div
          v-for="row in 4"
          :key="row"
          class="h-44 animate-pulse rounded-xl border border-line bg-surface"
        />
      </div>
      <div class="col-span-4 flex flex-col gap-4">
        <div
          v-for="row in 3"
          :key="row"
          class="h-40 animate-pulse rounded-xl border border-line bg-surface"
        />
      </div>
    </div>

    <PageSection v-else-if="notFound || error" class="mt-4" body-class="py-4">
      <EmptyState
        :title="notFound ? '镜像不存在' : '加载镜像详情失败'"
        :description="notFound ? '该镜像可能已被删除，或 URL 中的 ID 不正确。' : error"
        icon="alert"
      >
        <template #action>
          <div class="flex items-center gap-2">
            <AppButton @click="loadDetail()">重试</AppButton>
            <AppButton variant="primary" @click="router.push({ name: 'images' })">
              返回镜像列表
            </AppButton>
          </div>
        </template>
      </EmptyState>
    </PageSection>

    <template v-else-if="image">
      <div
        v-if="verifyError"
        role="alert"
        class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
      >
        <span>{{ verifyError }}</span>
        <AppButton size="sm" variant="ghost" @click="verifyError = ''">关闭</AppButton>
      </div>

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <div class="col-span-8 flex flex-col gap-4">
          <ImageSummaryCard :image="image" />
          <VerificationReport :verification="verification ?? image.latest_verification ?? null" />
          <ImageChunksTable :chunks="image.chunks ?? []" />
          <ImageChannelsTable :channels="image.channels ?? []" />
        </div>

        <div class="col-span-4 flex flex-col gap-4">
          <ManifestPanel :image-id="image.id" />
          <template v-if="canManage">
            <PublishPanel
              :image-id="image.id"
              :image-name="imageLabel"
              :status="image.status"
              @published="onPublished"
            />
            <AssignmentPanel
              :image-id="image.id"
              :image-name="imageLabel"
              :status="image.status"
              :rooms="rooms"
              :rooms-loading="roomsLoading"
              :rooms-error="roomsError"
            />
          </template>
        </div>
      </div>

      <SubmitManifestModal
        :open="submitOpen"
        :image-id="image.id"
        :image-name="imageLabel"
        :total-size="image.total_size"
        @close="submitOpen = false"
        @submitted="onManifestSubmitted"
      />
    </template>
  </div>
</template>
