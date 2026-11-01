<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'

import { ApiError, api, isForbidden } from '@/api'
import type { Area } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import AreaFormModal from '@/features/areas/AreaFormModal.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatRelative, shortId } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const PAGE_SIZE = 20

const auth = useAuthStore()
const toast = useToastStore()
const router = useRouter()

const items = ref<Area[]>([])
const allAreas = ref<Area[]>([])
const total = ref(0)
const page = ref(1)
const keyword = ref('')
const includeInactive = ref(false)
const loading = ref(false)
const error = ref('')

const formOpen = ref(false)
const editing = ref<Area | null>(null)
const deleteTarget = ref<Area | null>(null)
const deleting = ref(false)

const canManage = computed(() => auth.can(PERMISSIONS.areaManage))
const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))

const parentNames = computed(() => {
  const map = new Map<string, string>()
  for (const area of allAreas.value) map.set(area.id, area.name)
  return map
})

function parentLabel(parentId: string | null | undefined): string {
  if (!parentId) return '—'
  return parentNames.value.get(parentId) ?? shortId(parentId)
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const data = await api.listAreas({
      page: page.value,
      page_size: PAGE_SIZE,
      keyword: keyword.value.trim() || undefined,
      include_inactive: includeInactive.value || undefined,
    })
    items.value = data.items ?? []
    total.value = data.total ?? 0
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少权限：当前账号没有区域读取权限（area:read）。'
      : errorMessage(caught, '加载区域列表失败')
  } finally {
    loading.value = false
  }
}

/** 父区域下拉/名称回填需要完整列表，忽略失败以免阻塞主列表。 */
async function loadOptions() {
  try {
    const data = await api.listAreas({ page: 1, page_size: 100, include_inactive: true })
    allAreas.value = data.items ?? []
  } catch {
    allAreas.value = []
  }
}

const { run: debouncedSearch } = useDebounceFn(() => {
  page.value = 1
  void load()
}, 320)

watch(keyword, () => debouncedSearch())
watch(includeInactive, () => {
  page.value = 1
  void load()
})
watch(page, () => void load())

function openCreate() {
  editing.value = null
  formOpen.value = true
}

function openDetail(area: Area) {
  void router.push({ name: 'area-detail', params: { id: area.id } })
}

function openEdit(area: Area) {
  editing.value = area
  formOpen.value = true
}

function onSaved() {
  formOpen.value = false
  void load()
  void loadOptions()
}

function askDelete(area: Area) {
  deleteTarget.value = area
}

async function confirmDelete() {
  const target = deleteTarget.value
  if (!target) return
  deleting.value = true
  try {
    await api.deleteArea(target.id)
    toast.success('区域已删除', target.name)
    deleteTarget.value = null
    if (items.value.length === 1 && page.value > 1) {
      page.value -= 1
    } else {
      void load()
    }
    void loadOptions()
  } catch (caught) {
    if (caught instanceof ApiError && caught.status === 409) {
      toast.error('无法删除区域', '该区域下仍有房间，请先迁移或删除房间后再试。')
    } else if (isForbidden(caught)) {
      toast.error('缺少权限', '当前账号没有区域管理权限（area:manage）。')
    } else {
      toast.error('删除失败', errorMessage(caught, '请稍后重试'))
    }
  } finally {
    deleting.value = false
  }
}

onMounted(() => {
  void load()
  void loadOptions()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="区域" description="建筑/楼层/片区与区域级 webhook，点击区域进入区域总览。">
      <template #actions>
        <SearchInput v-model="keyword" placeholder="搜索区域名称或编码" />
        <AppButton v-if="canManage" icon="plus" variant="primary" @click="openCreate">
          新建区域
        </AppButton>
      </template>
    </PageHeader>

    <div class="mt-4 flex items-center justify-between gap-3">
      <label class="flex items-center gap-2 text-[11.5px] text-muted">
        <input
          v-model="includeInactive"
          type="checkbox"
          class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
        />
        显示已停用区域
      </label>
      <span class="text-[11px] text-faint">共 {{ total }} 个区域</span>
    </div>

    <div
      v-if="error && items.length"
      role="alert"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <section class="mt-4 overflow-hidden rounded-xl border border-line bg-surface">
      <table v-if="items.length" class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">区域</th>
            <th class="table-th">父区域</th>
            <th class="table-th">时区</th>
            <th class="table-th">Webhook</th>
            <th class="table-th">状态</th>
            <th class="table-th">更新时间</th>
            <th class="table-th pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="area in items"
            :key="area.id"
            class="cursor-pointer border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            :class="area.is_active ? '' : 'opacity-70'"
            @click="openDetail(area)"
          >
            <td class="table-td pl-5">
              <RouterLink
                :to="{ name: 'area-detail', params: { id: area.id } }"
                class="text-[12px] font-medium text-ink transition-colors hover:text-accent"
                @click.stop
              >
                {{ area.name }}
              </RouterLink>
              <div class="mt-0.5 flex items-center gap-2 font-mono text-[10.5px] text-faint">
                <span>{{ area.code }}</span>
                <span v-if="area.description" class="truncate font-sans">
                  · {{ area.description }}
                </span>
              </div>
            </td>
            <td class="table-td">{{ parentLabel(area.parent_id) }}</td>
            <td class="table-td font-mono text-[11px]">{{ area.timezone }}</td>
            <td class="table-td">
              <span
                v-if="area.webhook_url"
                class="block max-w-[220px] truncate font-mono text-[10.5px] text-ink-soft"
                :title="area.webhook_url"
              >
                {{ area.webhook_url }}
              </span>
              <span v-else class="text-faint">—</span>
            </td>
            <td class="table-td">
              <StateBadge
                :label="area.is_active ? '启用' : '已停用'"
                :tone="area.is_active ? 'emerald' : 'slate'"
              />
            </td>
            <td class="table-td text-[11px] text-faint">
              {{ formatRelative(area.updated_at) }}
            </td>
            <td class="table-td pr-5" @click.stop>
              <div class="flex items-center justify-end gap-1">
                <IconButton
                  icon="overview"
                  label="查看区域总览"
                  size="sm"
                  @click.stop="openDetail(area)"
                />
                <IconButton
                  v-if="canManage"
                  icon="pencil"
                  label="编辑区域"
                  size="sm"
                  @click.stop="openEdit(area)"
                />
                <IconButton
                  v-if="canManage"
                  icon="trash"
                  label="删除区域"
                  size="sm"
                  @click.stop="askDelete(area)"
                />
              </div>
            </td>
          </tr>
        </tbody>
      </table>

      <div v-else-if="loading" class="space-y-3 px-5 py-6">
        <div v-for="row in 5" :key="row" class="h-9 animate-pulse rounded-md bg-canvas" />
      </div>

      <EmptyState
        v-else-if="!error"
        title="还没有区域"
        description="区域是房间与课表的上层组织单位，先创建一个区域再添加房间。"
        icon="layers"
      >
        <template v-if="canManage" #action>
          <AppButton icon="plus" variant="primary" @click="openCreate">新建区域</AppButton>
        </template>
      </EmptyState>

      <EmptyState v-else title="加载区域失败" :description="error" icon="alert">
        <template #action>
          <AppButton icon="refresh" @click="load()">重试</AppButton>
        </template>
      </EmptyState>

      <PaginationBar
        v-if="!loading && items.length"
        :page="page"
        :total-pages="totalPages"
        :total="total"
        label="个区域"
        @change="(next) => (page = next)"
      />
    </section>

    <AreaFormModal
      :open="formOpen"
      :area="editing"
      :areas="allAreas"
      @close="formOpen = false"
      @saved="onSaved"
    />

    <ConfirmDialog
      :open="deleteTarget !== null"
      title="删除区域"
      :description="`确认删除区域「${deleteTarget?.name ?? ''}」吗？`"
      detail="删除后不可恢复。若该区域下仍有房间，后端会拒绝删除（409）。"
      confirm-label="删除区域"
      danger
      :loading="deleting"
      @close="deleteTarget = null"
      @confirm="confirmDelete"
    />
  </div>
</template>
