<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type { Announcement } from '@/api/schema'
import type { AnnouncementListParams } from '@/api/types'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import FilterTabs from '@/components/ui/FilterTabs.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import AnnouncementFormModal from '@/features/announcements/AnnouncementFormModal.vue'
import AnnouncementPreviewModal from '@/features/announcements/AnnouncementPreviewModal.vue'
import { STATUS_FILTER_TABS, statusMeta } from '@/features/announcements/announcementStatus'
import { useDebounceFn } from '@/composables/useDebounceFn'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatDateTime, formatFullDateTime, formatRelative } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const PAGE_SIZE = 20

const auth = useAuthStore()
const toast = useToastStore()

const announcements = ref<Announcement[]>([])
const total = ref(0)
const page = ref(1)
const status = ref('')
const keyword = ref('')
const loading = ref(false)
const error = ref('')

const formOpen = ref(false)
const editTarget = ref<Announcement | null>(null)
const previewOpen = ref(false)
const previewTarget = ref<Announcement | null>(null)

const deleteOpen = ref(false)
const deleteTarget = ref<Announcement | null>(null)
const deleting = ref(false)

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const canWrite = computed(() => auth.can(PERMISSIONS.announcementWrite))
const canPublish = computed(() => auth.can(PERMISSIONS.announcementPublish))
const canDelete = computed(() => auth.can(PERMISSIONS.announcementDelete))

async function load() {
  loading.value = true
  error.value = ''
  try {
    const result = await api.listAdminAnnouncements({
      page: page.value,
      page_size: PAGE_SIZE,
      keyword: keyword.value.trim() || undefined,
      status: status.value
        ? (status.value as NonNullable<AnnouncementListParams['status']>)
        : undefined,
    })
    announcements.value = result.items ?? []
    total.value = result.total ?? 0
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少 announcement:read 权限，无法读取公告列表。'
      : errorMessage(caught, '加载公告列表失败')
    announcements.value = []
    total.value = 0
  } finally {
    loading.value = false
  }
}

const { run: debouncedSearch } = useDebounceFn(() => {
  page.value = 1
  void load()
}, 300)

watch(keyword, () => debouncedSearch())

function applyStatus() {
  page.value = 1
  void load()
}

function changePage(next: number) {
  page.value = next
  void load()
}

function openCreate() {
  editTarget.value = null
  formOpen.value = true
}

function openEdit(announcement: Announcement) {
  editTarget.value = announcement
  formOpen.value = true
}

function openPreview(announcement: Announcement) {
  previewTarget.value = announcement
  previewOpen.value = true
}

function onSaved(_announcement: Announcement, created: boolean) {
  formOpen.value = false
  toast.success(created ? '公告已创建' : '公告已更新')
  void load()
}

function confirmDelete(announcement: Announcement) {
  deleteTarget.value = announcement
  deleteOpen.value = true
}

async function removeAnnouncement() {
  if (!deleteTarget.value?.id) return
  deleting.value = true
  try {
    await api.deleteAnnouncement(deleteTarget.value.id)
    deleteOpen.value = false
    toast.success('公告已删除', deleteTarget.value.title)
    deleteTarget.value = null
    void load()
  } catch (caught) {
    toast.error('删除公告失败', errorMessage(caught, '请稍后重试'))
  } finally {
    deleting.value = false
  }
}

onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="公告" description="草稿、发布、归档与置顶；正文只读纯文本展示。">
      <template #actions>
        <SearchInput v-model="keyword" class="w-[232px]" placeholder="搜索公告标题 / 正文" />
        <IconButton
          icon="refresh"
          label="刷新公告列表"
          variant="outline"
          :loading="loading"
          @click="load()"
        />
        <AppButton v-if="canWrite" icon="plus" variant="primary" @click="openCreate">
          新建公告
        </AppButton>
      </template>
    </PageHeader>

    <div class="mt-4 flex flex-wrap items-center justify-between gap-3">
      <FilterTabs v-model="status" :tabs="STATUS_FILTER_TABS" @change="applyStatus" />
      <p class="text-[11px] text-muted">
        共 {{ total }} 条公告
        <span v-if="!canPublish"> · 当前账号无发布权限，只能保存草稿或编辑已有公告</span>
      </p>
    </div>

    <div
      v-if="error"
      class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <section class="mt-3 overflow-hidden rounded-xl border border-line bg-surface">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">标题</th>
            <th class="table-th">状态</th>
            <th class="table-th">置顶</th>
            <th class="table-th">作者</th>
            <th class="table-th">更新时间</th>
            <th class="table-th pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <template v-if="loading">
            <tr
              v-for="row in 5"
              :key="`skeleton-${row}`"
              class="border-b border-line/70 last:border-b-0"
            >
              <td v-for="col in 6" :key="col" class="table-td">
                <span class="block h-3 animate-pulse rounded bg-canvas" />
              </td>
            </tr>
          </template>

          <template v-else>
            <tr
              v-for="announcement in announcements"
              :key="announcement.id"
              class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            >
              <td class="table-td max-w-[420px] pl-5">
                <button
                  type="button"
                  class="flex max-w-full items-center gap-1.5 text-left text-[12px] text-ink transition-colors hover:text-accent"
                  :title="announcement.title"
                  @click="openPreview(announcement)"
                >
                  <span v-if="announcement.is_pinned" class="shrink-0 text-accent">
                    <AppIcon name="pin" :size="12" />
                  </span>
                  <span class="truncate">{{ announcement.title }}</span>
                </button>
                <p class="mt-0.5 truncate text-[10.5px] text-faint">
                  {{ announcement.content }}
                </p>
              </td>
              <td class="table-td">
                <StateBadge
                  :label="statusMeta(announcement.status).label"
                  :tone="statusMeta(announcement.status).tone"
                />
              </td>
              <td class="table-td">
                <StateBadge v-if="announcement.is_pinned" label="置顶" tone="accent" :dot="false" />
                <span v-else class="text-[11px] text-faint">—</span>
              </td>
              <td class="table-td text-[11.5px]">{{ announcement.author_name || '—' }}</td>
              <td class="table-td whitespace-nowrap">
                <span
                  class="text-[11px] text-muted"
                  :title="formatFullDateTime(announcement.updated_at)"
                >
                  {{ formatRelative(announcement.updated_at) }}
                </span>
                <p class="text-[10px] text-faint">{{ formatDateTime(announcement.updated_at) }}</p>
              </td>
              <td class="table-td pr-5">
                <div class="flex items-center justify-end gap-1">
                  <IconButton
                    icon="eye"
                    label="预览公告"
                    size="sm"
                    @click="openPreview(announcement)"
                  />
                  <IconButton
                    v-if="canWrite"
                    icon="pencil"
                    label="编辑公告"
                    size="sm"
                    @click="openEdit(announcement)"
                  />
                  <IconButton
                    v-if="canDelete"
                    icon="trash"
                    label="删除公告"
                    size="sm"
                    @click="confirmDelete(announcement)"
                  />
                </div>
              </td>
            </tr>
          </template>
        </tbody>
      </table>

      <EmptyState
        v-if="!loading && !announcements.length"
        :title="status || keyword ? '没有匹配的公告' : '还没有公告'"
        :description="
          canWrite
            ? '创建公告草稿，拥有发布权限后可发布或归档。'
            : '当前账号只能查看公告；需要 announcement:write 才能创建。'
        "
        icon="megaphone"
      >
        <template v-if="canWrite" #action>
          <AppButton icon="plus" variant="primary" @click="openCreate">新建公告</AppButton>
        </template>
      </EmptyState>

      <PaginationBar
        v-if="total > 0"
        :page="page"
        :total-pages="totalPages"
        :total="total"
        label="条公告"
        @change="changePage"
      />
    </section>

    <AnnouncementFormModal
      :open="formOpen"
      :announcement="editTarget"
      :can-publish="canPublish"
      @close="formOpen = false"
      @saved="onSaved"
    />
    <AnnouncementPreviewModal
      :open="previewOpen"
      :announcement="previewTarget"
      @close="previewOpen = false"
    />

    <ConfirmDialog
      :open="deleteOpen"
      title="删除公告"
      :description="`确认删除公告「${deleteTarget?.title || ''}」？`"
      detail="删除后公开接口将立即不再返回该公告，且操作不可撤销。"
      confirm-label="删除公告"
      danger
      :loading="deleting"
      @close="deleteOpen = false"
      @confirm="removeAnnouncement"
    />
  </div>
</template>
