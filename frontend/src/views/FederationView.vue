<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'

import { api } from '@/api'
import type { FederationNodeView, FederationOverview } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import EmptyState from '@/components/ui/EmptyState.vue'
import PageHeader from '@/components/ui/PageHeader.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import MetricCard from '@/features/areas/MetricCard.vue'
import type { MetricCardProps } from '@/features/areas/MetricCard.vue'
import {
  formatCount,
  nodeStatusMeta,
  summaryNumbers,
  validateNodeBaseUrl,
} from '@/features/federation/federationMeta'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, formatRelative } from '@/utils/format'

/**
 * 总控阶段 2：多控制节点联邦（只读聚合 MVP）。
 * 本节点 + 已注册的其他控制节点摘要；支持添加/刷新/删除节点。
 */

const toast = useToastStore()

const overview = ref<FederationOverview | null>(null)
const loading = ref(false)
const error = ref('')

const addOpen = ref(false)
const addBusy = ref(false)
const addError = ref('')
const addForm = ref({ name: '', base_url: '', token: '' })

const deleteTarget = ref<FederationNodeView | null>(null)
const deleteBusy = ref(false)
const refreshingId = ref('')

const nodes = computed(() => overview.value?.nodes ?? [])

const kpis = computed<MetricCardProps[]>(() => {
  const totals = overview.value?.totals
  return [
    {
      label: '联邦节点',
      value: totals ? `${totals.nodes_online}/${totals.nodes_total}` : '—',
      note: totals ? `离线 ${totals.nodes_offline} · 含本节点` : '',
      badge: totals && totals.nodes_offline > 0 ? '有离线' : undefined,
      tone: totals && totals.nodes_offline > 0 ? 'amber' : 'emerald',
      icon: 'server',
      loading: loading.value,
    },
    {
      label: '房间总数',
      value: formatCount(totals?.rooms),
      note: '在线节点合计',
      tone: 'sky',
      icon: 'door',
      loading: loading.value,
    },
    {
      label: '机器 / 在线',
      value: totals
        ? `${formatCount(totals.machines)} / ${formatCount(totals.online_machines)}`
        : '—',
      note: '在线节点合计',
      tone: 'violet',
      icon: 'cpu',
      loading: loading.value,
    },
    {
      label: '未结束工单',
      value: formatCount(totals?.open_tickets),
      note: 'open / in_progress',
      tone: 'amber',
      icon: 'ticket',
      loading: loading.value,
    },
    {
      label: '镜像制品',
      value: formatCount(totals?.images),
      note: '含已发布/未发布',
      tone: 'slate',
      icon: 'package',
      loading: loading.value,
    },
  ]
})

const localSummaryNumbers = computed(() => summaryNumbers(overview.value?.local))

async function load() {
  loading.value = true
  error.value = ''
  try {
    overview.value = await api.federationOverview()
  } catch (caught) {
    error.value = errorMessage(caught, '总控概览加载失败')
  } finally {
    loading.value = false
  }
}

function openAdd() {
  addForm.value = { name: '', base_url: '', token: '' }
  addError.value = ''
  addOpen.value = true
}

async function submitAdd() {
  const name = addForm.value.name.trim()
  const baseUrlError = validateNodeBaseUrl(addForm.value.base_url)
  if (!name) {
    addError.value = '请填写节点名称'
    return
  }
  if (baseUrlError) {
    addError.value = baseUrlError
    return
  }
  if (!addForm.value.token.trim()) {
    addError.value = '请填写联邦 token（远端节点的 FEDERATION_TOKEN）'
    return
  }
  addBusy.value = true
  addError.value = ''
  try {
    await api.createFederationNode({
      name,
      base_url: addForm.value.base_url.trim(),
      token: addForm.value.token.trim(),
    })
    addOpen.value = false
    toast.success('联邦节点已添加', name)
    await load()
  } catch (caught) {
    addError.value = errorMessage(caught, '添加联邦节点失败')
  } finally {
    addBusy.value = false
  }
}

function askDelete(node: FederationNodeView) {
  deleteTarget.value = node
}

async function confirmDelete() {
  const node = deleteTarget.value
  deleteTarget.value = null
  if (!node) return
  deleteBusy.value = true
  try {
    await api.deleteFederationNode(node.id)
    toast.success('联邦节点已删除', node.name)
    await load()
  } catch (caught) {
    toast.error('删除联邦节点失败', errorMessage(caught, '请稍后重试'))
  } finally {
    deleteBusy.value = false
  }
}

async function refreshNode(node: FederationNodeView) {
  if (refreshingId.value) return
  refreshingId.value = node.id
  try {
    await api.refreshFederationNode(node.id)
    toast.success('节点已刷新', node.name)
  } catch (caught) {
    toast.error('节点刷新失败', errorMessage(caught, '请检查节点地址与 token'))
  } finally {
    refreshingId.value = ''
    await load()
  }
}

onMounted(() => void load())
</script>

<template>
  <div class="mx-auto max-w-[1440px] px-7 py-5">
    <PageHeader
      title="总控联邦"
      description="跨控制节点只读聚合（阶段 2 MVP）：本节点 + 已注册的控制节点摘要"
    >
      <template #actions>
        <AppButton icon="refresh" :loading="loading" @click="load">刷新</AppButton>
        <AppButton variant="primary" icon="plus" @click="openAdd">添加节点</AppButton>
      </template>
    </PageHeader>

    <div v-if="error" class="mt-4 rounded-xl border border-line bg-surface">
      <EmptyState title="总控概览加载失败" :description="error" icon="alert">
        <template #action>
          <AppButton size="sm" @click="load">重试</AppButton>
        </template>
      </EmptyState>
    </div>

    <template v-else>
      <section class="mt-4 grid grid-cols-5 gap-3" aria-label="联邦聚合 KPI">
        <MetricCard v-for="card in kpis" :key="card.label" v-bind="card" />
      </section>

      <div class="mt-4 grid grid-cols-12 items-start gap-4">
        <PageSection
          class="col-span-4"
          title="本节点"
          description="当前控制节点（此页面运行所在节点）"
        >
          <div class="px-5 py-4">
            <div class="flex items-center justify-between gap-2">
              <p class="truncate text-[12px] font-medium text-ink">
                {{ overview?.node_name ?? '—' }}
              </p>
              <StateBadge label="本节点" tone="accent" />
            </div>
            <p class="mt-1 text-[10.5px] text-faint">
              摘要时间：{{ overview ? formatFullDateTime(overview.generated_at) : '—' }}
            </p>
            <div class="mt-3 grid grid-cols-2 gap-2">
              <div
                v-for="item in localSummaryNumbers"
                :key="item.label"
                class="rounded-lg border border-line bg-canvas/40 px-3 py-2"
              >
                <p class="text-[10.5px] text-faint">{{ item.label }}</p>
                <p class="mt-0.5 text-[15px] font-semibold tabular-nums text-ink">
                  {{ formatCount(item.value) }}
                </p>
              </div>
            </div>
          </div>
        </PageSection>

        <PageSection
          class="col-span-8"
          title="联邦节点"
          description="已注册的其他控制节点；刷新会实时拉取远端摘要（5s 超时）"
        >
          <ul v-if="nodes.length" class="divide-y divide-line/70">
            <li v-for="node in nodes" :key="node.id" class="px-5 py-3.5">
              <div class="flex flex-wrap items-start justify-between gap-2">
                <div class="min-w-0">
                  <div class="flex items-center gap-2">
                    <p class="truncate text-[12px] font-medium text-ink">{{ node.name }}</p>
                    <StateBadge
                      :label="nodeStatusMeta(node.status).label"
                      :tone="nodeStatusMeta(node.status).tone"
                    />
                  </div>
                  <p class="mt-0.5 truncate font-mono text-[10.5px] text-faint">
                    {{ node.base_url }}
                  </p>
                </div>
                <div class="flex items-center gap-1.5">
                  <AppButton
                    size="sm"
                    icon="refresh"
                    :loading="refreshingId === node.id"
                    :disabled="Boolean(refreshingId)"
                    @click="refreshNode(node)"
                  >
                    刷新
                  </AppButton>
                  <AppButton
                    size="sm"
                    variant="danger"
                    icon="trash"
                    :disabled="deleteBusy"
                    @click="askDelete(node)"
                  >
                    删除
                  </AppButton>
                </div>
              </div>

              <div
                class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-[10.5px] text-muted"
              >
                <span>
                  最近刷新：
                  {{ node.last_seen_at ? formatRelative(node.last_seen_at) : '未刷新' }}
                </span>
                <span v-for="item in summaryNumbers(node.summary)" :key="item.label">
                  {{ item.label }}
                  <span class="font-mono text-ink-soft">{{ formatCount(item.value) }}</span>
                </span>
              </div>
            </li>
          </ul>
          <EmptyState
            v-else
            title="还没有联邦节点"
            description="点击右上角「添加节点」，填入另一个控制节点的地址与 FEDERATION_TOKEN 即可注册。"
            icon="server"
          >
            <template #action>
              <AppButton size="sm" variant="primary" icon="plus" @click="openAdd">
                添加节点
              </AppButton>
            </template>
          </EmptyState>
        </PageSection>
      </div>
    </template>

    <AppModal
      :open="addOpen"
      title="添加联邦节点"
      description="注册另一个控制节点；添加后点「刷新」拉取摘要（该节点需配置相同的 FEDERATION_TOKEN）"
      size="md"
      @close="addOpen = false"
    >
      <div class="flex flex-col gap-3">
        <label class="flex flex-col gap-1 text-[11px] text-muted">
          节点名称
          <input
            v-model="addForm.name"
            placeholder="机房 B · 控制节点"
            class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
          />
        </label>
        <label class="flex flex-col gap-1 text-[11px] text-muted">
          节点地址（http/https，不带路径）
          <input
            v-model="addForm.base_url"
            placeholder="http://127.0.0.1:8081"
            class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
          />
        </label>
        <label class="flex flex-col gap-1 text-[11px] text-muted">
          联邦 token（远端节点的 FEDERATION_TOKEN）
          <input
            v-model="addForm.token"
            type="password"
            placeholder="远端 FEDERATION_TOKEN"
            class="h-9 rounded-md border border-line bg-surface px-3 text-[12px] text-ink outline-none focus:border-accent"
          />
        </label>
        <p v-if="addError" class="text-[11px] text-rose-600">{{ addError }}</p>
      </div>
      <template #footer>
        <AppButton :disabled="addBusy" @click="addOpen = false">取消</AppButton>
        <AppButton variant="primary" :loading="addBusy" @click="submitAdd">添加</AppButton>
      </template>
    </AppModal>

    <ConfirmDialog
      :open="deleteTarget !== null"
      title="删除联邦节点"
      :description="deleteTarget ? `确认删除联邦节点「${deleteTarget.name}」？` : ''"
      detail="仅删除本节点保存的联邦记录，不会影响远端控制节点本身。"
      :danger="true"
      :loading="deleteBusy"
      confirm-label="删除"
      @close="deleteTarget = null"
      @confirm="confirmDelete"
    />
  </div>
</template>
