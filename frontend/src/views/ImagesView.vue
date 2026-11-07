<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import { api } from '@/api'
import type { ArtifactStatus, ImageArtifact } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import CreateImageModal from '@/features/images/CreateImageModal.vue'
import ImageLifecycleActions from '@/features/images/ImageLifecycleActions.vue'
import {
  ARTIFACT_STATUS_OPTIONS,
  IMAGE_FORMAT_LABELS,
  IMAGE_KIND_OPTIONS,
  artifactStatusMeta,
  compressionLabel,
  imageKindLabel,
} from '@/features/images/labels'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatBytes, formatDateTime } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const router = useRouter()
const auth = useAuthStore()
const toast = useToastStore()

const PAGE_SIZE = 20

const items = ref<ImageArtifact[]>([])
const page = ref(1)
const total = ref(0)
const loading = ref(false)
const error = ref('')

const kind = ref('')
const status = ref('')
const name = ref('')

const createOpen = ref(false)

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const canManage = computed(() => auth.can(PERMISSIONS.imageManage))

async function load() {
  loading.value = true
  error.value = ''
  try {
    const data = await api.listImages({
      kind: kind.value || undefined,
      status: (status.value || undefined) as ArtifactStatus | undefined,
      name: name.value.trim() || undefined,
      page: page.value,
      page_size: PAGE_SIZE,
    })
    items.value = data.items ?? []
    total.value = data.total ?? 0
    if (data.page) page.value = data.page
  } catch (caught) {
    error.value = errorMessage(caught, '加载镜像列表失败')
    items.value = []
    total.value = 0
  } finally {
    loading.value = false
  }
}

function reloadFromFirstPage() {
  page.value = 1
  void load()
}

function changePage(next: number) {
  page.value = next
  void load()
}

const searchLater = useDebounceFn(reloadFromFirstPage, 350)
watch(name, () => searchLater.run())

function openImage(id: string) {
  void router.push({ name: 'image-detail', params: { id } })
}

function onCreated(artifact: ImageArtifact) {
  void router.push({ name: 'image-detail', params: { id: artifact.id } })
}

function openCreate() {
  if (!canManage.value) {
    toast.error('缺少权限', '当前账号没有 image:manage 权限')
    return
  }
  createOpen.value = true
}

/** 下架成功后原地更新状态（后端返回最新 artifact），无需整页刷新。 */
function onImageDeprecated(artifact: ImageArtifact) {
  items.value = items.value.map((item) =>
    item.id === artifact.id ? { ...item, ...artifact } : item,
  )
}

onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="镜像"
      :badge="total ? `${total} 个制品` : ''"
      description="镜像制品、manifest、验证、发布与房间分配。"
    >
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新镜像列表"
          variant="outline"
          :loading="loading"
          @click="load"
        />
        <AppButton v-if="canManage" variant="primary" icon="plus" @click="openCreate">
          新建镜像
        </AppButton>
      </template>
    </PageHeader>

    <div class="mt-4 flex flex-wrap items-center gap-2">
      <select
        v-model="kind"
        class="select w-[150px]"
        aria-label="按类型筛选"
        @change="reloadFromFirstPage"
      >
        <option value="">全部类型</option>
        <option v-for="option in IMAGE_KIND_OPTIONS" :key="option.value" :value="option.value">
          {{ option.label }}
        </option>
      </select>

      <select
        v-model="status"
        class="select w-[156px]"
        aria-label="按状态筛选"
        @change="reloadFromFirstPage"
      >
        <option value="">全部状态</option>
        <option v-for="option in ARTIFACT_STATUS_OPTIONS" :key="option.value" :value="option.value">
          {{ option.label }}
        </option>
      </select>

      <SearchInput
        v-model="name"
        class="w-[260px]"
        placeholder="按名称搜索镜像"
        label="按名称搜索镜像"
      />

      <div class="flex-1" />

      <span class="text-[11px] text-faint">共 {{ total }} 个镜像制品</span>
    </div>

    <div
      v-if="error"
      role="alert"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load">重试</AppButton>
    </div>

    <section class="mt-4 overflow-hidden rounded-xl border border-line bg-surface">
      <div class="overflow-x-auto">
        <table class="w-full border-collapse text-left">
          <thead>
            <tr class="border-b border-line bg-canvas/50">
              <th class="table-th pl-5">名称:版本</th>
              <th class="table-th">类型</th>
              <th class="table-th">平台</th>
              <th class="table-th">格式 / 压缩</th>
              <th class="table-th">状态</th>
              <th class="table-th">总大小</th>
              <th class="table-th" :class="canManage ? '' : 'pr-5'">更新时间</th>
              <th v-if="canManage" class="table-th pr-5">操作</th>
            </tr>
          </thead>

          <tbody v-if="loading">
            <tr v-for="row in 6" :key="row" class="border-b border-line/70 last:border-b-0">
              <td
                v-for="cell in canManage ? 8 : 7"
                :key="cell"
                class="table-td"
                :class="cell === 1 ? 'pl-5' : cell === (canManage ? 8 : 7) ? 'pr-5' : ''"
              >
                <div class="h-3 w-24 animate-pulse rounded bg-line" />
              </td>
            </tr>
          </tbody>

          <tbody v-else-if="items.length">
            <tr
              v-for="image in items"
              :key="image.id"
              class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/60"
              @click="openImage(image.id)"
            >
              <td class="table-td pl-5">
                <div class="min-w-0">
                  <p class="truncate font-medium text-ink">{{ image.name }}:{{ image.version }}</p>
                  <p class="mt-0.5 font-mono text-[10px] text-faint">{{ image.id.slice(0, 8) }}</p>
                </div>
              </td>
              <td class="table-td">{{ imageKindLabel(image.kind) }}</td>
              <td class="table-td font-mono text-[10.5px]">{{ image.platform }}</td>
              <td class="table-td text-[11px]">
                {{ IMAGE_FORMAT_LABELS[image.format] }} / {{ compressionLabel(image.compression) }}
              </td>
              <td class="table-td">
                <StateBadge
                  :label="artifactStatusMeta(image.status).label"
                  :tone="artifactStatusMeta(image.status).tone"
                />
              </td>
              <td class="table-td text-[11px]">{{ formatBytes(image.total_size) }}</td>
              <td class="table-td text-[11px]" :class="canManage ? '' : 'pr-5'">
                {{ formatDateTime(image.updated_at) }}
              </td>
              <td v-if="canManage" class="table-td pr-5" @click.stop>
                <ImageLifecycleActions
                  :image="image"
                  size="sm"
                  @deprecated="onImageDeprecated"
                  @deleted="load"
                  @refresh="load"
                />
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <EmptyState
        v-if="!loading && !items.length && !error"
        title="暂无镜像制品"
        description="创建镜像后提交 manifest 并执行同步验证，通过后即可发布到通道并分配到房间。"
        icon="package"
      >
        <template v-if="canManage" #action>
          <AppButton variant="primary" icon="plus" @click="openCreate">新建镜像</AppButton>
        </template>
      </EmptyState>

      <PaginationBar
        v-if="!loading && total > 0"
        :page="page"
        :total-pages="totalPages"
        :total="total"
        label="个镜像"
        @change="changePage"
      />
    </section>

    <CreateImageModal :open="createOpen" @close="createOpen = false" @created="onCreated" />
  </div>
</template>
