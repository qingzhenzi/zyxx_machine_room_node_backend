<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api, isForbidden } from '@/api'
import type { AgentConfig, Area, Room, UpdateAgentConfigReply } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import AgentConfigFormModal from '@/features/agentConfigs/AgentConfigFormModal.vue'
import AgentConfigTable from '@/features/agentConfigs/AgentConfigTable.vue'
import EffectivePolicyPanel from '@/features/agentConfigs/EffectivePolicyPanel.vue'
import { useAuthStore } from '@/stores/auth'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { PERMISSIONS } from '@/utils/permissions'

const auth = useAuthStore()
const toast = useToastStore()

const configs = ref<AgentConfig[]>([])
const areas = ref<Area[]>([])
const rooms = ref<Room[]>([])
const loading = ref(false)
const scopesLoading = ref(false)
const error = ref('')

const formOpen = ref(false)
const editTarget = ref<AgentConfig | null>(null)

const canRead = computed(() => auth.can(PERMISSIONS.deviceRead))
const canManage = computed(() => auth.can(PERMISSIONS.deviceManage))

async function load() {
  if (!canRead.value) {
    configs.value = []
    return
  }
  loading.value = true
  error.value = ''
  try {
    const reply = await api.listAgentConfigs()
    configs.value = reply.items ?? []
  } catch (caught) {
    error.value = isForbidden(caught)
      ? '缺少 device:read 权限，无法读取节点配置。'
      : errorMessage(caught, '加载节点配置失败')
    configs.value = []
  } finally {
    loading.value = false
  }
}

async function loadScopes() {
  scopesLoading.value = true
  try {
    const [roomPage, areaPage] = await Promise.all([
      api.listRooms({ page_size: 100 }),
      api.listAreas({ page_size: 100 }),
    ])
    rooms.value = roomPage.items ?? []
    areas.value = areaPage.items ?? []
  } catch (caught) {
    if (!error.value) error.value = errorMessage(caught, '加载房间 / 区域列表失败')
  } finally {
    scopesLoading.value = false
  }
}

function openCreate() {
  editTarget.value = null
  formOpen.value = true
}

function openEdit(config: AgentConfig) {
  editTarget.value = config
  formOpen.value = true
}

function onSaved(reply: UpdateAgentConfigReply) {
  formOpen.value = false
  toast.success(
    '节点配置已保存',
    `revision r${reply.config.revision} · 已下发到 ${reply.pushed_rooms} 个房间`,
  )
  void load()
}

function refreshAll() {
  void load()
  void loadScopes()
}

onMounted(refreshAll)
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="节点配置"
      description="global → area → room 作用域策略、二次下发与生效策略预览。"
    >
      <template #actions>
        <IconButton
          icon="refresh"
          label="刷新节点配置"
          variant="outline"
          :loading="loading || scopesLoading"
          @click="refreshAll"
        />
        <AppButton v-if="canManage" icon="plus" variant="primary" @click="openCreate">
          新建 / 更新配置
        </AppButton>
      </template>
    </PageHeader>

    <div
      v-if="error"
      class="mt-4 flex items-center justify-between rounded-lg border border-rose-200 bg-rose-50 px-4 py-2.5 text-[11.5px] text-rose-700"
    >
      <span>{{ error }}</span>
      <AppButton size="sm" variant="ghost" @click="refreshAll">重试</AppButton>
    </div>

    <p class="mt-4 text-[11px] text-muted">
      共 {{ configs.length }} 份作用域配置
      <span v-if="!canManage"> · 当前账号只读（写入需要 device:manage 权限）</span>
      <span v-else> · 保存后自动向绑定设备槽位的房间下发 config.update</span>
    </p>

    <section v-if="!canRead" class="mt-2 overflow-hidden rounded-xl border border-line bg-surface">
      <EmptyState
        title="缺少 device:read 权限"
        description="节点配置列表与生效策略预览需要 device:read 权限；已绑定 device:manage 的账号才能写入配置。"
        icon="lock"
      />
    </section>

    <div v-else class="mt-2 grid grid-cols-12 items-start gap-4">
      <div class="col-span-8">
        <AgentConfigTable
          :configs="configs"
          :areas="areas"
          :rooms="rooms"
          :loading="loading"
          :can-manage="canManage"
          @create="openCreate"
          @edit="openEdit"
        />
        <p class="mt-2 text-[10.5px] leading-5 text-faint">
          说明：配置按作用域保存为增量 patch，未显式覆盖的字段继承上层；全球配置的 scope_id
          必须为空， 区域 / 房间作用域必须指定目标。revision 每次写入递增。
        </p>
      </div>

      <div class="col-span-4">
        <EffectivePolicyPanel :rooms="rooms" />
      </div>
    </div>

    <AgentConfigFormModal
      :open="formOpen"
      :config="editTarget"
      :areas="areas"
      :rooms="rooms"
      @close="formOpen = false"
      @saved="onSaved"
    />
  </div>
</template>
