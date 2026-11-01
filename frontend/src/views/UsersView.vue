<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'

import { api, isForbidden } from '@/api'
import type { RoleSummary, UserSummary } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PaginationBar from '@/components/ui/PaginationBar.vue'
import SearchInput from '@/components/ui/SearchInput.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useDebounceFn } from '@/composables/useDebounceFn'
import UserFormModal from '@/features/users/UserFormModal.vue'
import UserRolesModal from '@/features/users/UserRolesModal.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatDateTime, shortId } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

const PAGE_SIZE = 20

const auth = useAuthStore()
const toast = useToastStore()

const users = ref<UserSummary[]>([])
const roles = ref<RoleSummary[]>([])
const total = ref(0)
const page = ref(1)
const keyword = ref('')
const loading = ref(false)
const error = ref('')

const formOpen = ref(false)
const rolesOpen = ref(false)
const editTarget = ref<UserSummary | null>(null)
const rolesTarget = ref<UserSummary | null>(null)

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)))
const roleNames = computed(
  () => new Map(roles.value.map((role) => [role.code ?? '', role.name || role.code || ''])),
)

function isSelf(user: UserSummary): boolean {
  return !!user.id && user.id === auth.user?.id
}

function roleLabel(code: string): string {
  return roleNames.value.get(code) || code
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const result = await api.listUsers({
      page: page.value,
      page_size: PAGE_SIZE,
      keyword: keyword.value.trim() || undefined,
    })
    users.value = result.items ?? []
    total.value = result.total ?? 0
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少 user:manage 权限，无法读取用户列表。'
      : errorMessage(caught, '加载用户列表失败')
    users.value = []
    total.value = 0
  } finally {
    loading.value = false
  }
}

async function loadRoles() {
  try {
    roles.value = await api.listRoles()
  } catch {
    roles.value = []
  }
}

const { run: debouncedSearch } = useDebounceFn(() => {
  page.value = 1
  void load()
}, 300)

watch(keyword, () => debouncedSearch())

function changePage(next: number) {
  page.value = next
  void load()
}

function openCreate() {
  editTarget.value = null
  formOpen.value = true
}

function openEdit(user: UserSummary) {
  editTarget.value = user
  formOpen.value = true
}

function openRoles(user: UserSummary) {
  rolesTarget.value = user
  rolesOpen.value = true
}

function onFormSaved() {
  const created = editTarget.value === null
  formOpen.value = false
  toast.success(created ? '用户已创建' : '用户已更新')
  void load()
  if (created) void loadRoles()
}

function onRolesSaved() {
  rolesOpen.value = false
  toast.success('角色已更新')
  void load()
  void auth.refreshProfile()
}

onMounted(() => {
  void load()
  void loadRoles()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="用户" description="管理后台账号、启用状态与角色分配。">
      <template #actions>
        <SearchInput
          v-model="keyword"
          class="w-[248px]"
          placeholder="搜索用户名 / 显示名称 / 邮箱"
        />
        <IconButton
          icon="refresh"
          label="刷新用户列表"
          variant="outline"
          :loading="loading"
          @click="load()"
        />
        <AppButton
          v-if="auth.can(PERMISSIONS.userManage)"
          icon="plus"
          variant="primary"
          @click="openCreate"
        >
          新建用户
        </AppButton>
      </template>
    </PageHeader>

    <div
      v-if="error"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <p class="mt-4 text-[11px] text-muted">
      共 {{ total }} 个账号 · 停用自己或最后一个管理员会被后端安全约束拒绝
    </p>

    <section class="mt-2 overflow-hidden rounded-xl border border-line bg-surface">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">用户名</th>
            <th class="table-th">显示名称</th>
            <th class="table-th">邮箱</th>
            <th class="table-th">角色</th>
            <th class="table-th">状态</th>
            <th class="table-th">创建时间</th>
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
              <td v-for="col in 7" :key="col" class="table-td">
                <span class="block h-3 animate-pulse rounded bg-canvas" />
              </td>
            </tr>
          </template>

          <template v-else>
            <tr
              v-for="user in users"
              :key="user.id || user.username"
              class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
              :class="isSelf(user) ? 'bg-accent-soft/30' : ''"
            >
              <td class="table-td pl-5">
                <div class="flex items-center gap-2">
                  <span class="font-mono text-[11.5px] text-ink">{{ user.username }}</span>
                  <span
                    v-if="isSelf(user)"
                    class="rounded bg-accent-soft px-1.5 py-px text-[9.5px] font-medium text-accent"
                  >
                    我
                  </span>
                  <span class="font-mono text-[9.5px] text-faint" :title="user.id">
                    {{ shortId(user.id, 6) }}
                  </span>
                </div>
              </td>
              <td class="table-td">{{ user.display_name || '—' }}</td>
              <td class="table-td">
                <span v-if="user.email" class="text-[11.5px]">{{ user.email }}</span>
                <span v-else class="text-[11px] text-faint">未绑定</span>
              </td>
              <td class="table-td">
                <div v-if="user.roles?.length" class="flex flex-wrap gap-1">
                  <span
                    v-for="code in user.roles"
                    :key="code"
                    class="rounded bg-canvas px-1.5 py-0.5 text-[10px] text-ink-soft"
                    :title="code"
                  >
                    {{ roleLabel(code) }}
                  </span>
                </div>
                <span v-else class="text-[11px] text-faint">—</span>
              </td>
              <td class="table-td">
                <StateBadge
                  :label="user.is_active ? '启用' : '停用'"
                  :tone="user.is_active ? 'emerald' : 'slate'"
                />
              </td>
              <td class="table-td whitespace-nowrap text-[11px] text-muted">
                {{ formatDateTime(user.created_at) }}
              </td>
              <td class="table-td pr-5">
                <div class="flex items-center justify-end gap-1">
                  <IconButton icon="pencil" label="编辑用户" size="sm" @click="openEdit(user)" />
                  <IconButton icon="shield" label="设置角色" size="sm" @click="openRoles(user)" />
                </div>
              </td>
            </tr>
          </template>
        </tbody>
      </table>

      <EmptyState
        v-if="!loading && !users.length"
        title="没有匹配的用户"
        description="调整搜索关键词，或创建新的后台账号。"
        icon="users"
      />

      <PaginationBar
        v-if="total > 0"
        :page="page"
        :total-pages="totalPages"
        :total="total"
        label="个账号"
        @change="changePage"
      />
    </section>

    <UserFormModal
      :open="formOpen"
      :user="editTarget"
      :roles="roles"
      @close="formOpen = false"
      @saved="onFormSaved"
    />
    <UserRolesModal
      :open="rolesOpen"
      :user="rolesTarget"
      :roles="roles"
      @close="rolesOpen = false"
      @saved="onRolesSaved"
    />
  </div>
</template>
