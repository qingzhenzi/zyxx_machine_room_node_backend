<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'

import { ApiError, api, isForbidden } from '@/api'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import FormField from '@/components/ui/FormField.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import {
  canDeleteIso,
  canUnshareIso,
  isIsoShared,
  isoShareMeta,
  isoShareRequestTime,
  isoSourceLabel,
  isoStatusMeta,
  seedScanSummary,
  shortSha,
} from '@/features/isos/types'
import type { IsoAsset, IsoSeedScanReply } from '@/features/isos/types'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatBytes, formatDateTime } from '@/utils/format'

const PAGE_SIZE = 20

const auth = useAuthStore()
const toast = useToastStore()

/** 管理台 ISO 库：列表所有人可读（image:read）；审批/扫描仅管理员。 */
const isAdmin = computed(() => auth.roles.includes('admin'))

/** 撤销共享/删除的入口：管理员或 ISO 上传者本人（接口侧同样会校验）。 */
function canManageIso(iso: IsoAsset): boolean {
  return isAdmin.value || iso.uploader_user_id === auth.user?.id
}

/** 无操作按钮时的说明文案（预设由 seed 目录管理、上传中不可操作）。 */
function emptyActionLabel(iso: IsoAsset): string {
  if (iso.is_preset) return '预设由 seed 目录管理'
  if (iso.status === 'uploading') return '上传中'
  return '—'
}

const items = ref<IsoAsset[]>([])
const pending = ref<IsoAsset[]>([])
const loading = ref(false)
const pendingLoading = ref(false)
const error = ref('')
const pendingError = ref('')
const keyword = ref('')
const page = ref(1)

const scanResult = ref<IsoSeedScanReply | null>(null)
const scanning = ref(false)
const scanError = ref('')

const reviewTarget = ref<IsoAsset | null>(null)
const reviewApprove = ref(true)
const reviewNote = ref('')
const reviewing = ref(false)
const reviewError = ref('')

const unshareTarget = ref<IsoAsset | null>(null)
const unsharing = ref(false)

const deleteTarget = ref<IsoAsset | null>(null)
const deleting = ref(false)

const filteredItems = computed(() => {
  const needle = keyword.value.trim().toLowerCase()
  if (!needle) return items.value
  return items.value.filter((iso) =>
    [iso.filename, iso.display_name, iso.uploader_name ?? ''].some((value) =>
      value.toLowerCase().includes(needle),
    ),
  )
})

const totalPages = computed(() => Math.max(1, Math.ceil(filteredItems.value.length / PAGE_SIZE)))

const pagedItems = computed(() => {
  const start = (page.value - 1) * PAGE_SIZE
  return filteredItems.value.slice(start, start + PAGE_SIZE)
})

const stats = computed(() => ({
  total: items.value.length,
  presets: items.value.filter((iso) => iso.is_preset).length,
  shared: items.value.filter((iso) => !iso.is_preset && isIsoShared(iso)).length,
  pending: pending.value.length,
}))

watch(keyword, () => {
  page.value = 1
})

watch(totalPages, (next) => {
  if (page.value > next) page.value = next
})

async function loadList() {
  loading.value = true
  error.value = ''
  try {
    items.value = await api.listIsos({ status: 'ready' })
  } catch (caught) {
    items.value = []
    error.value = isForbidden(caught)
      ? '缺少 image:read 权限，无法查看 ISO 库。'
      : errorMessage(caught, '加载 ISO 列表失败')
  } finally {
    loading.value = false
  }
}

async function loadPending() {
  if (!isAdmin.value) {
    pending.value = []
    pendingError.value = ''
    return
  }
  pendingLoading.value = true
  pendingError.value = ''
  try {
    pending.value = await api.listIsoShareRequests()
  } catch (caught) {
    pending.value = []
    pendingError.value = isForbidden(caught)
      ? '缺少管理员权限，无法读取待审批共享列表。'
      : errorMessage(caught, '加载待审批共享失败')
  } finally {
    pendingLoading.value = false
  }
}

function refresh() {
  void Promise.all([loadList(), loadPending()])
}

async function runSeedScan() {
  if (!isAdmin.value || scanning.value) return

  scanning.value = true
  scanError.value = ''
  scanResult.value = null
  try {
    const reply = await api.scanIsoSeed()
    scanResult.value = reply
    toast.success('预设扫描完成', seedScanSummary(reply))
    refresh()
  } catch (caught) {
    scanError.value = errorMessage(caught, '触发预设扫描失败')
    toast.error('预设扫描失败', scanError.value)
  } finally {
    scanning.value = false
  }
}

function openReview(iso: IsoAsset, approve: boolean) {
  reviewTarget.value = iso
  reviewApprove.value = approve
  reviewNote.value = ''
  reviewError.value = ''
}

function closeReview() {
  if (reviewing.value) return
  reviewTarget.value = null
}

/** 审批弹窗里切换「批准/拒绝」，保留已填写的备注。 */
function toggleReviewDecision() {
  reviewApprove.value = !reviewApprove.value
}

function changePage(next: number) {
  page.value = next
}

async function submitReview() {
  const target = reviewTarget.value
  if (!target || reviewing.value) return

  reviewing.value = true
  reviewError.value = ''
  try {
    const note = reviewNote.value.trim()
    await api.reviewIsoShare(target.id, {
      approve: reviewApprove.value,
      note: note || undefined,
    })
    toast.success(
      reviewApprove.value ? '已批准共享' : '已拒绝共享',
      note ? `${target.filename} · ${note}` : target.filename,
    )
    reviewTarget.value = null
    refresh()
  } catch (caught) {
    if (caught instanceof ApiError && caught.status === 409) {
      reviewError.value = `${errorMessage(caught, '共享申请状态已变化')}（409：可能已被审批或撤销，请刷新后重试）`
      refresh()
    } else {
      reviewError.value = errorMessage(caught, '审批共享申请失败')
    }
    toast.error('审批失败', reviewError.value)
  } finally {
    reviewing.value = false
  }
}

async function confirmUnshare() {
  const target = unshareTarget.value
  if (!target || unsharing.value) return

  unsharing.value = true
  try {
    await api.unshareIso(target.id)
    toast.success('已撤销共享', `${target.filename} 已恢复为私有`)
    unshareTarget.value = null
    refresh()
  } catch (caught) {
    unshareTarget.value = null
    const message = errorMessage(caught, '撤销共享失败')
    toast.error(
      caught instanceof ApiError && caught.status === 409 ? '共享状态已变化' : '撤销共享失败',
      message,
    )
    refresh()
  } finally {
    unsharing.value = false
  }
}

async function confirmDelete() {
  const target = deleteTarget.value
  if (!target || deleting.value) return

  deleting.value = true
  try {
    await api.deleteIso(target.id)
    toast.success('ISO 已删除', target.filename)
    deleteTarget.value = null
    refresh()
  } catch (caught) {
    deleteTarget.value = null
    const message = errorMessage(caught, '删除 ISO 失败')
    if (caught instanceof ApiError && caught.status === 409) {
      // 409：被进行中的构建会话引用，或状态已变化。
      toast.error('无法删除该 ISO', `${message}；若正被构建会话使用，请先结束会话`)
    } else if (caught instanceof ApiError && caught.status === 403) {
      toast.error('无权删除该 ISO', message)
    } else {
      toast.error('删除失败', message)
    }
    refresh()
  } finally {
    deleting.value = false
  }
}

onMounted(refresh)
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="ISO 库"
      :badge="stats.total ? `${stats.total} 个可用 ISO` : ''"
      :description="
        isAdmin
          ? '平台安装 ISO 资产：预设扫描、共享审批与删除。'
          : '平台安装 ISO 资产：预设与已共享 ISO 供镜像制作使用（共享审批仅管理员）。'
      "
    >
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新 ISO 库"
          variant="outline"
          :loading="loading || pendingLoading"
          @click="refresh"
        />
        <AppButton
          v-if="isAdmin"
          variant="primary"
          icon="rotate-cw"
          :loading="scanning"
          title="扫描预设目录，把新增 ISO 导入平台库"
          @click="runSeedScan"
        >
          触发预设扫描
        </AppButton>
      </template>
    </PageHeader>

    <!-- seed 扫描结果 -->
    <div
      v-if="scanResult"
      class="mt-4 rounded-lg border border-emerald-200 bg-emerald-50 px-4 py-2.5 text-[11.5px] text-emerald-800"
    >
      <div class="flex items-center justify-between gap-3">
        <span class="font-medium">预设扫描完成：{{ seedScanSummary(scanResult) }}</span>
        <button class="text-emerald-700 hover:underline" @click="scanResult = null">关闭</button>
      </div>
      <ul v-if="scanResult.errors?.length" class="mt-1.5 space-y-1 text-[11px] text-rose-700">
        <li v-for="item in scanResult.errors.slice(0, 5)" :key="item">· {{ item }}</li>
      </ul>
    </div>

    <div
      v-if="scanError"
      role="alert"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ scanError }}</span>
      <AppButton v-if="isAdmin" size="sm" variant="ghost" @click="runSeedScan">重试</AppButton>
    </div>

    <!-- 统计 -->
    <div class="mt-4 grid grid-cols-4 gap-3">
      <div class="rounded-xl border border-line bg-surface px-4 py-3">
        <p class="text-[10.5px] text-faint">可用 ISO</p>
        <p class="mt-1 text-[20px] font-semibold tracking-[-0.02em] text-ink">
          {{ stats.total }}
        </p>
      </div>
      <div class="rounded-xl border border-line bg-surface px-4 py-3">
        <p class="text-[10.5px] text-faint">系统预设</p>
        <p class="mt-1 text-[20px] font-semibold tracking-[-0.02em] text-ink">
          {{ stats.presets }}
        </p>
      </div>
      <div class="rounded-xl border border-line bg-surface px-4 py-3">
        <p class="text-[10.5px] text-faint">已共享</p>
        <p class="mt-1 text-[20px] font-semibold tracking-[-0.02em] text-ink">
          {{ stats.shared }}
        </p>
      </div>
      <div class="rounded-xl border border-line bg-surface px-4 py-3">
        <p class="text-[10.5px] text-faint">待审批</p>
        <p
          class="mt-1 text-[20px] font-semibold tracking-[-0.02em]"
          :class="stats.pending ? 'text-amber-600' : 'text-ink'"
        >
          {{ stats.pending }}
        </p>
      </div>
    </div>

    <!-- 待审批共享（admin） -->
    <PageSection
      v-if="isAdmin"
      class="mt-5"
      title="待审批共享"
      :description="`教师申请把自上传 ISO 共享给所有教师；批准后 visibility=shared。当前 ${stats.pending} 条待处理。`"
    >
      <div
        v-if="pendingError"
        class="m-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
      >
        <span>{{ pendingError }}</span>
        <AppButton size="sm" variant="ghost" @click="loadPending">重试</AppButton>
      </div>

      <div v-else-if="pendingLoading" class="space-y-2 p-4">
        <div v-for="row in 2" :key="row" class="h-10 animate-pulse rounded bg-canvas" />
      </div>

      <p v-else-if="!pending.length" class="px-5 py-6 text-center text-[11.5px] text-muted">
        暂无待审批共享申请。
      </p>

      <div v-else class="overflow-x-auto">
        <table class="w-full border-collapse text-left">
          <thead>
            <tr class="border-b border-line bg-canvas/50">
              <th class="table-th pl-5">文件名</th>
              <th class="table-th">大小</th>
              <th class="table-th">上传者</th>
              <th class="table-th">申请时间</th>
              <th class="table-th">备注</th>
              <th class="table-th pr-5">操作</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="iso in pending"
              :key="iso.id"
              class="border-b border-line/70 last:border-b-0"
            >
              <td class="table-td pl-5">
                <p class="font-medium text-ink">{{ iso.display_name || iso.filename }}</p>
                <p class="mt-0.5 font-mono text-[10px] text-faint">{{ iso.filename }}</p>
              </td>
              <td class="table-td text-[11px]">{{ formatBytes(iso.size_bytes) }}</td>
              <td class="table-td text-[11px]">{{ isoSourceLabel(iso) }}</td>
              <td class="table-td text-[11px]">{{ formatDateTime(isoShareRequestTime(iso)) }}</td>
              <td class="table-td max-w-[240px] truncate text-[11px] text-muted">
                {{ iso.share_note || '—' }}
              </td>
              <td class="table-td pr-5">
                <div class="flex items-center gap-1.5">
                  <AppButton
                    size="sm"
                    variant="primary"
                    icon="check"
                    @click="openReview(iso, true)"
                  >
                    批准
                  </AppButton>
                  <AppButton size="sm" variant="danger" icon="x" @click="openReview(iso, false)">
                    拒绝
                  </AppButton>
                </div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </PageSection>

    <!-- 错误 / 列表 -->
    <div
      v-if="error"
      role="alert"
      class="mt-5 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="loadList">重试</AppButton>
    </div>

    <section class="mt-5 overflow-hidden rounded-xl border border-line bg-surface">
      <div class="flex flex-wrap items-center gap-2 border-b border-line px-4 py-2.5">
        <SearchInput
          v-model="keyword"
          class="w-[280px]"
          placeholder="搜索文件名 / 显示名 / 上传者"
          label="搜索 ISO"
        />
        <div class="flex-1" />
        <span class="text-[11px] text-faint">
          共 {{ filteredItems.length }} 个可用 ISO（≤16 GiB）
        </span>
      </div>

      <div class="overflow-x-auto">
        <table class="w-full border-collapse text-left">
          <thead>
            <tr class="border-b border-line bg-canvas/50">
              <th class="table-th pl-5">文件</th>
              <th class="table-th">大小</th>
              <th class="table-th">SHA-256</th>
              <th class="table-th">来源</th>
              <th class="table-th">可见性 / 共享</th>
              <th class="table-th">状态</th>
              <th class="table-th">更新时间</th>
              <th class="table-th pr-5">操作</th>
            </tr>
          </thead>

          <tbody v-if="loading">
            <tr v-for="row in 5" :key="row" class="border-b border-line/70 last:border-b-0">
              <td
                v-for="cell in 8"
                :key="cell"
                class="table-td"
                :class="cell === 1 ? 'pl-5' : cell === 8 ? 'pr-5' : ''"
              >
                <div class="h-3 w-24 animate-pulse rounded bg-line" />
              </td>
            </tr>
          </tbody>

          <tbody v-else-if="pagedItems.length">
            <tr
              v-for="iso in pagedItems"
              :key="iso.id"
              class="border-b border-line/70 last:border-b-0 hover:bg-canvas/50"
            >
              <td class="table-td pl-5">
                <div class="min-w-0">
                  <p class="truncate font-medium text-ink">
                    {{ iso.display_name || iso.filename }}
                    <StateBadge v-if="iso.is_preset" label="预设" tone="accent" class="ml-1" />
                  </p>
                  <p class="mt-0.5 truncate font-mono text-[10px] text-faint">{{ iso.filename }}</p>
                </div>
              </td>
              <td class="table-td text-[11px]">{{ formatBytes(iso.size_bytes) }}</td>
              <td class="table-td font-mono text-[10.5px] text-muted">
                {{ shortSha(iso.sha256) }}
              </td>
              <td class="table-td text-[11px]">{{ isoSourceLabel(iso) }}</td>
              <td class="table-td">
                <StateBadge
                  :label="isoShareMeta(iso).label"
                  :tone="isoShareMeta(iso).tone"
                  :title="isoShareMeta(iso).hint"
                />
              </td>
              <td class="table-td">
                <StateBadge
                  :label="isoStatusMeta(iso.status).label"
                  :tone="isoStatusMeta(iso.status).tone"
                />
              </td>
              <td class="table-td text-[11px]">
                {{ formatDateTime(iso.uploaded_at || iso.updated_at) }}
              </td>
              <td class="table-td pr-5">
                <div v-if="canManageIso(iso)" class="flex items-center gap-1.5">
                  <AppButton
                    v-if="canUnshareIso(iso)"
                    size="sm"
                    icon="eye-off"
                    title="撤销共享（恢复为私有）"
                    @click="unshareTarget = iso"
                  >
                    撤销共享
                  </AppButton>
                  <AppButton
                    v-if="canDeleteIso(iso)"
                    size="sm"
                    variant="danger"
                    icon="trash"
                    title="删除 ISO（非预设；不可恢复）"
                    @click="deleteTarget = iso"
                  >
                    删除
                  </AppButton>
                </div>
                <span v-else class="text-[10.5px] text-faint">{{ emptyActionLabel(iso) }}</span>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <EmptyState
        v-if="!loading && !filteredItems.length && !error"
        title="暂无可用 ISO"
        :description="
          keyword
            ? '没有匹配当前搜索的 ISO，试试调整关键词。'
            : '上传 ISO 或由管理员把安装镜像放入预设目录后触发「预设扫描」导入。'
        "
        icon="archive"
      >
        <template v-if="isAdmin && !keyword" #action>
          <AppButton variant="primary" icon="rotate-cw" :loading="scanning" @click="runSeedScan">
            触发预设扫描
          </AppButton>
        </template>
      </EmptyState>

      <PaginationBar
        v-if="!loading && filteredItems.length > PAGE_SIZE"
        :page="page"
        :total-pages="totalPages"
        :total="filteredItems.length"
        label="个 ISO"
        @change="changePage"
      />
    </section>

    <!-- 审批弹窗 -->
    <AppModal
      :open="reviewTarget !== null"
      :title="reviewApprove ? '批准共享申请' : '拒绝共享申请'"
      :description="
        reviewTarget
          ? `「${reviewTarget.display_name || reviewTarget.filename}」由 ${isoSourceLabel(reviewTarget)} 申请共享。`
          : ''
      "
      @close="closeReview"
    >
      <div class="space-y-3">
        <FormField label="审批备注（可选）" input-id="iso-review-note">
          <textarea
            id="iso-review-note"
            v-model="reviewNote"
            class="textarea min-h-[88px]"
            maxlength="500"
            placeholder="例如：教学用镜像，批准共享；或拒绝原因"
          />
        </FormField>
        <p class="text-[11px] leading-5 text-muted">
          <template v-if="reviewApprove">
            批准后该 ISO 变为「已共享」，所有具备 image:read 的教师都可以在镜像制作中选择它。
          </template>
          <template v-else>
            拒绝后可修改备注并再次申请；当前审批意见会记录在 ISO 的共享备注中。
          </template>
        </p>
        <p v-if="reviewError" class="text-[11px] leading-5 text-rose-600">{{ reviewError }}</p>
      </div>

      <template #footer>
        <AppButton :disabled="reviewing" @click="closeReview">取消</AppButton>
        <AppButton
          :variant="reviewApprove ? 'danger' : 'secondary'"
          :disabled="reviewing"
          @click="toggleReviewDecision"
        >
          改为{{ reviewApprove ? '拒绝' : '批准' }}
        </AppButton>
        <AppButton
          :variant="reviewApprove ? 'primary' : 'danger'"
          :loading="reviewing"
          @click="submitReview"
        >
          {{ reviewApprove ? '确认批准' : '确认拒绝' }}
        </AppButton>
      </template>
    </AppModal>

    <ConfirmDialog
      :open="unshareTarget !== null"
      title="撤销共享"
      :description="`确认撤销「${unshareTarget?.display_name || unshareTarget?.filename || ''}」的共享？`"
      detail="撤销后该 ISO 恢复为私有：仅上传者与管理员可见，已在使用它的构建会话不受影响。"
      confirm-label="确认撤销"
      :loading="unsharing"
      @confirm="confirmUnshare"
      @close="unshareTarget = null"
    />

    <ConfirmDialog
      :open="deleteTarget !== null"
      title="删除 ISO"
      :description="`确认永久删除「${deleteTarget?.display_name || deleteTarget?.filename || ''}」？`"
      :detail="`将删除 ISO 记录与对象存储内容，不可恢复。预设 ISO 不能删除；被进行中的构建会话引用时会被服务端拒绝（409）。文件名：${deleteTarget?.filename || ''}`"
      confirm-label="确认删除"
      danger
      :loading="deleting"
      @confirm="confirmDelete"
      @close="deleteTarget = null"
    />
  </div>
</template>
