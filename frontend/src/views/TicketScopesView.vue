<script setup lang="ts">
/**
 * 工单可见范围（P1 Wave 2）：按用户配置可访问区域。
 * 未配置区域的用户默认可见全部工单；拥有 ticket:scope:all 的用户不受范围限制。
 */
import { computed, onMounted, ref } from 'vue'

import { api, isForbidden } from '@/api'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { shortId } from '@/utils/format'
import { PERMISSIONS } from '@/utils/permissions'

interface TicketScopeRow {
  user_id: string
  display_name?: string | null
  roles?: string[]
  area_ids?: string[]
}

interface AreaOption {
  id: string
  name: string
  code?: string
}

const auth = useAuthStore()
const toast = useToastStore()

const scopes = ref<TicketScopeRow[]>([])
const areas = ref<AreaOption[]>([])
const loading = ref(false)
const error = ref('')
const forbidden = ref(false)

const modalOpen = ref(false)
const editing = ref<TicketScopeRow | null>(null)
const selectedAreaIds = ref<string[]>([])
const scopeText = ref('')
const formError = ref('')
const saving = ref(false)

const canManage = computed(() =>
  auth.canAny(PERMISSIONS.ticketScopeManage, PERMISSIONS.ticketManage),
)

const areaNames = computed(() => {
  const map: Record<string, string> = {}
  for (const area of areas.value) map[area.id] = area.name
  return map
})

function scopeAreaLabels(scope: TicketScopeRow): string[] {
  const ids = scope.area_ids ?? []
  return ids.map((id) => areaNames.value[id] || shortId(id))
}

async function load() {
  loading.value = true
  error.value = ''
  forbidden.value = false
  try {
    scopes.value = await api.listTicketScopes()
  } catch (caught) {
    scopes.value = []
    if (isForbidden(caught)) {
      forbidden.value = true
    } else {
      error.value = errorMessage(caught, '加载工单可见范围失败')
    }
  } finally {
    loading.value = false
  }
}

/** 区域选项由工单范围接口提供，不依赖 area:read；失败时回退为手填 UUID。 */
async function loadAreas() {
  try {
    const data = await api.listTicketScopeAreas()
    areas.value = data.map((area) => ({ id: area.id, name: area.name, code: area.code }))
  } catch {
    areas.value = []
  }
}

function openEdit(scope: TicketScopeRow) {
  editing.value = scope
  selectedAreaIds.value = [...(scope.area_ids ?? [])]
  scopeText.value = ''
  formError.value = ''
  modalOpen.value = true
}

function toggleArea(id: string) {
  if (selectedAreaIds.value.includes(id)) {
    selectedAreaIds.value = selectedAreaIds.value.filter((item) => item !== id)
  } else {
    selectedAreaIds.value = [...selectedAreaIds.value, id]
  }
}

function parsedManualIds(): string[] {
  return scopeText.value
    .split(/[\s,，、]+/)
    .map((item) => item.trim())
    .filter(Boolean)
}

async function save() {
  const scope = editing.value
  if (!scope) return
  formError.value = ''
  const areaIds = areas.value.length ? selectedAreaIds.value : parsedManualIds()
  saving.value = true
  try {
    await api.setTicketScopes(scope.user_id, { area_ids: areaIds })
    toast.success(
      '可见范围已更新',
      areaIds.length ? `已授权 ${areaIds.length} 个区域` : '未配置区域 = 可见全部工单',
    )
    modalOpen.value = false
    await load()
  } catch (caught) {
    if (isForbidden(caught)) {
      formError.value = '缺少权限：需要 ticket:scope:manage 或 ticket:manage。'
    } else {
      formError.value = errorMessage(caught, '保存可见范围失败，请稍后重试')
    }
  } finally {
    saving.value = false
  }
}

function refresh() {
  void Promise.all([load(), loadAreas()])
}

onMounted(() => {
  void load()
  void loadAreas()
})
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="工单可见范围"
      description="按用户配置可访问区域：未配置区域的用户默认可见全部工单；ticket:scope:all 用户不受范围限制。"
    >
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新工单可见范围"
          variant="outline"
          :loading="loading"
          @click="refresh"
        />
      </template>
    </PageHeader>

    <div
      class="mt-4 rounded-lg border border-sky-200 bg-sky-50 px-4 py-2.5 text-[11px] leading-5 text-sky-800"
    >
      范围只影响工单列表 / 详情 / 统计 / 导出的可见性：配置了区域的用户只能看到该区域内工单，
      区域为空的工单仅对不受限用户可见。未配置任何区域的用户保持“可见全部”的兼容行为。
    </div>

    <div
      v-if="error && scopes.length"
      role="alert"
      class="mt-3 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="load()">重试</AppButton>
    </div>

    <section class="mt-3 overflow-hidden rounded-xl border border-line bg-surface">
      <table v-if="scopes.length" class="w-full border-collapse text-left">
        <thead>
          <tr class="border-b border-line bg-canvas/50">
            <th class="table-th min-w-[200px] pl-5">用户</th>
            <th class="table-th w-[200px]">角色</th>
            <th class="table-th">可见区域</th>
            <th class="table-th w-[110px] pr-5 text-right">操作</th>
          </tr>
        </thead>
        <tbody>
          <tr
            v-for="scope in scopes"
            :key="scope.user_id"
            class="border-b border-line/70 transition-colors last:border-b-0 hover:bg-canvas/50"
          >
            <td class="table-td pl-5">
              <div class="text-[12px] font-medium text-ink">
                {{ scope.display_name || '未知用户' }}
              </div>
              <div class="mt-0.5 font-mono text-[10px] text-faint">
                {{ shortId(scope.user_id) }}
              </div>
            </td>
            <td class="table-td">
              <div class="flex flex-wrap gap-1">
                <StateBadge
                  v-for="role in scope.roles ?? []"
                  :key="role"
                  :label="role"
                  tone="slate"
                  :dot="false"
                />
                <span v-if="!(scope.roles ?? []).length" class="text-[11px] text-faint">—</span>
              </div>
            </td>
            <td class="table-td">
              <div v-if="(scope.area_ids ?? []).length" class="flex flex-wrap gap-1">
                <span
                  v-for="(label, index) in scopeAreaLabels(scope)"
                  :key="`${scope.user_id}-${index}`"
                  class="rounded bg-canvas px-1.5 py-0.5 text-[10.5px] text-ink-soft"
                >
                  {{ label }}
                </span>
              </div>
              <StateBadge v-else label="全部可见（未配置区域）" tone="accent" :dot="false" />
            </td>
            <td class="table-td pr-5 text-right">
              <AppButton v-if="canManage" size="sm" icon="pencil" @click="openEdit(scope)">
                编辑范围
              </AppButton>
            </td>
          </tr>
        </tbody>
      </table>

      <div v-else-if="loading" class="space-y-3 px-5 py-6">
        <div v-for="row in 5" :key="row" class="h-9 animate-pulse rounded-md bg-canvas" />
      </div>

      <EmptyState
        v-else-if="forbidden"
        title="缺少可见范围权限"
        description="当前账号既没有 ticket:scope:manage 也没有 ticket:manage，无法管理工单可见范围。"
        icon="lock"
      />

      <EmptyState v-else-if="error" title="加载可见范围失败" :description="error" icon="alert">
        <template #action>
          <AppButton icon="refresh" @click="load()">重试</AppButton>
        </template>
      </EmptyState>

      <EmptyState
        v-else
        title="还没有可配置的用户"
        description="拥有 ticket:read 的启用用户会出现在这里；未配置区域的用户默认可见全部工单。"
        icon="users"
      />
    </section>

    <AppModal
      :open="modalOpen"
      title="编辑工单可见范围"
      :description="`用户：${editing?.display_name || editing?.user_id || ''}。不勾选任何区域 = 可见全部工单。`"
      size="lg"
      @close="modalOpen = false"
    >
      <div class="flex flex-col gap-4">
        <div v-if="areas.length">
          <p class="mb-2 text-[11.5px] font-medium text-ink-soft">授权区域</p>
          <div
            class="grid max-h-[300px] grid-cols-2 gap-1 overflow-y-auto rounded-md border border-line p-2"
          >
            <label
              v-for="area in areas"
              :key="area.id"
              class="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-[11.5px] transition-colors hover:bg-canvas"
            >
              <input
                type="checkbox"
                class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
                :checked="selectedAreaIds.includes(area.id)"
                @change="toggleArea(area.id)"
              />
              <span class="min-w-0 truncate text-ink-soft">{{ area.name }}</span>
              <span v-if="area.code" class="ml-auto shrink-0 font-mono text-[10px] text-faint">
                {{ area.code }}
              </span>
            </label>
          </div>
          <p class="mt-2 text-[10.5px] text-faint">
            已选 {{ selectedAreaIds.length }} 个区域；不勾选表示不做区域限制。
          </p>
        </div>

        <div v-else>
          <p class="text-[11.5px] text-amber-700">
            当前账号没有区域读取权限，无法列出区域；可直接填写区域 UUID（每行 / 逗号分隔）。
          </p>
          <textarea
            v-model="scopeText"
            class="textarea mt-2 min-h-[90px] font-mono"
            placeholder="逐行填写区域 UUID，留空表示可见全部工单"
          />
        </div>

        <p
          v-if="formError"
          role="alert"
          class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700"
        >
          {{ formError }}
        </p>
      </div>

      <template #footer>
        <AppButton :disabled="saving" @click="modalOpen = false">取消</AppButton>
        <AppButton variant="primary" :loading="saving" @click="save">保存范围</AppButton>
      </template>
    </AppModal>
  </div>
</template>
