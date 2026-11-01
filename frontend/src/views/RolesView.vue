<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api, isForbidden } from '@/api'
import type { Permission, RoleSummary } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import ProgressBar from '@/components/ui/ProgressBar.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import RoleFormModal from '@/features/roles/RoleFormModal.vue'
import RolePermissionsModal from '@/features/roles/RolePermissionsModal.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { PERMISSIONS } from '@/utils/permissions'

const auth = useAuthStore()
const toast = useToastStore()

const roles = ref<RoleSummary[]>([])
const permissions = ref<Permission[]>([])
const loading = ref(false)
const error = ref('')

const createOpen = ref(false)
const permissionsOpen = ref(false)
const permissionsTarget = ref<RoleSummary | null>(null)

const deleteOpen = ref(false)
const deleteTarget = ref<RoleSummary | null>(null)
const deleting = ref(false)

const systemCount = computed(() => roles.value.filter((role) => role.is_system).length)

function permissionCount(role: RoleSummary): number {
  return role.permissions?.length ?? 0
}

function permissionPercent(role: RoleSummary): number {
  if (permissions.value.length === 0) return 0
  return (permissionCount(role) / permissions.value.length) * 100
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const [roleList, permissionList] = await Promise.all([api.listRoles(), api.listPermissions()])
    roles.value = roleList
    permissions.value = permissionList
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少 role:manage 权限，无法读取角色与权限点。'
      : errorMessage(caught, '加载角色与权限点失败')
    roles.value = []
    permissions.value = []
  } finally {
    loading.value = false
  }
}

function openPermissions(role: RoleSummary) {
  permissionsTarget.value = role
  permissionsOpen.value = true
}

function onCreated() {
  createOpen.value = false
  toast.success('角色已创建')
  void load()
}

function onPermissionsSaved() {
  permissionsOpen.value = false
  toast.success('角色权限已保存')
  void load()
  void auth.refreshProfile()
}

function confirmDelete(role: RoleSummary) {
  deleteTarget.value = role
  deleteOpen.value = true
}

async function removeRole() {
  if (!deleteTarget.value?.id) return
  deleting.value = true
  try {
    await api.deleteRole(deleteTarget.value.id)
    deleteOpen.value = false
    toast.success('角色已删除', deleteTarget.value.name || deleteTarget.value.code)
    deleteTarget.value = null
    void load()
    void auth.refreshProfile()
  } catch (caught) {
    toast.error('删除角色失败', errorMessage(caught, '请稍后重试'))
  } finally {
    deleting.value = false
  }
}

onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader title="角色与权限" description="角色 CRUD、权限点分组配置与系统角色约束。">
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新角色列表"
          variant="outline"
          :loading="loading"
          @click="load()"
        />
        <AppButton
          v-if="auth.can(PERMISSIONS.roleManage)"
          icon="plus"
          variant="primary"
          @click="createOpen = true"
        >
          新建角色
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
      共 {{ roles.length }} 个角色（{{ systemCount }} 个系统内置 ·
      {{ roles.length - systemCount }} 个自定义） · {{ permissions.length }} 个权限点 ·
      系统角色不可删除且权限只读
    </p>

    <section class="mt-2 overflow-hidden rounded-xl border border-line bg-surface">
      <table class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th pl-5">角色</th>
            <th class="table-th">描述</th>
            <th class="table-th">权限</th>
            <th class="table-th">类型</th>
            <th class="table-th pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <template v-if="loading">
            <tr
              v-for="row in 3"
              :key="`skeleton-${row}`"
              class="border-b border-line/70 last:border-b-0"
            >
              <td v-for="col in 5" :key="col" class="table-td">
                <span class="block h-3 animate-pulse rounded bg-canvas" />
              </td>
            </tr>
          </template>

          <template v-else>
            <tr
              v-for="role in roles"
              :key="role.id || role.code"
              class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
            >
              <td class="table-td pl-5">
                <div class="flex items-center gap-1.5">
                  <span class="text-[12px] font-medium text-ink">{{ role.name || '—' }}</span>
                  <span class="font-mono text-[10px] text-faint">{{ role.code }}</span>
                </div>
              </td>
              <td class="table-td">
                <span class="line-clamp-2 max-w-[360px] text-[11.5px] text-muted">
                  {{ role.description || '—' }}
                </span>
              </td>
              <td class="table-td">
                <div class="w-[132px]">
                  <p class="mb-1 text-[11px] text-ink-soft">
                    {{ permissionCount(role) }} / {{ permissions.length }} 个权限
                  </p>
                  <ProgressBar
                    :value="permissionPercent(role)"
                    :tone="role.is_system ? 'line' : 'accent'"
                  />
                </div>
              </td>
              <td class="table-td">
                <StateBadge
                  :label="role.is_system ? '系统内置' : '自定义'"
                  :tone="role.is_system ? 'violet' : 'slate'"
                />
              </td>
              <td class="table-td pr-5">
                <div class="flex items-center justify-end gap-1">
                  <AppButton size="sm" @click="openPermissions(role)">
                    {{ role.is_system ? '查看权限' : '编辑权限' }}
                  </AppButton>
                  <IconButton
                    icon="trash"
                    :label="role.is_system ? '系统内置角色不可删除' : '删除角色'"
                    size="sm"
                    :disabled="role.is_system"
                    @click="confirmDelete(role)"
                  />
                </div>
              </td>
            </tr>
          </template>
        </tbody>
      </table>

      <EmptyState
        v-if="!loading && !roles.length"
        title="还没有角色"
        description="创建自定义角色并勾选权限点，然后分配给用户。"
        icon="shield"
      />
    </section>

    <RoleFormModal
      :open="createOpen"
      :permissions="permissions"
      @close="createOpen = false"
      @saved="onCreated"
    />
    <RolePermissionsModal
      :open="permissionsOpen"
      :role="permissionsTarget"
      :permissions="permissions"
      @close="permissionsOpen = false"
      @saved="onPermissionsSaved"
    />

    <ConfirmDialog
      :open="deleteOpen"
      title="删除角色"
      :description="`确认删除角色「${deleteTarget?.name || deleteTarget?.code || ''}」？`"
      :detail="`角色编码 ${deleteTarget?.code ?? '—'}。删除后，使用该角色的账号将失去对应权限；系统内置角色不可删除。`"
      confirm-label="删除角色"
      danger
      :loading="deleting"
      @close="deleteOpen = false"
      @confirm="removeRole"
    />
  </div>
</template>
