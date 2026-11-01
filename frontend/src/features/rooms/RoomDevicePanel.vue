<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { ApiError, api, isForbidden } from '@/api'
import type { DeviceTokenReply, RoomDevice, RoomMachine } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppIcon from '@/components/ui/AppIcon.vue'
import AppModal from '@/components/ui/AppModal.vue'
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue'
import CopyText from '@/components/ui/CopyText.vue'
import FormField from '@/components/ui/FormField.vue'
import IconButton from '@/components/ui/IconButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { deviceOnlineMeta, deviceStatusMeta } from '@/features/devices/deviceMeta'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime, formatRelative, shortId } from '@/utils/format'

const props = defineProps<{
  roomId: string
  device: RoomDevice | null
  loading: boolean
  canManage: boolean
}>()

const emit = defineEmits<{
  changed: []
}>()

const toast = useToastStore()

const bindSlot = ref('')
const bindBusy = ref(false)
const unbindOpen = ref(false)
const unbindBusy = ref(false)
const tokenKind = ref<'auto' | 'approval'>('auto')
const tokenTtl = ref('900')
const tokenBusy = ref(false)
const tokenError = ref('')
const tokenResult = ref<DeviceTokenReply | null>(null)

// ---- 子机（一个房间多台机器，P1）----
const machines = ref<RoomMachine[]>([])
const machinesLoading = ref(false)
const machinesError = ref('')
const machineFormOpen = ref(false)
const machineLabel = ref('')
const machineKind = ref<'auto' | 'approval'>('auto')
const machineTtl = ref('900')
const machineTokenBusy = ref(false)
const machineTokenError = ref('')
const machineToken = ref<DeviceTokenReply | null>(null)
const machineRemoveTarget = ref<RoomMachine | null>(null)
const machineRemoveBusy = ref(false)
let machinesRequest = 0

const statusMeta = computed(() => deviceStatusMeta(props.device?.status))
const onlineMeta = computed(() => deviceOnlineMeta(props.device?.online))
const hasBoundDevice = computed(() => Boolean(props.device?.serial))
const childMachines = computed(() => machines.value.filter((machine) => !machine.is_primary))
const machineRemoveDescription = computed(() =>
  machineRemoveTarget.value ? `确认解绑子机「${machineTitle(machineRemoveTarget.value)}」吗？` : '',
)

watch(
  () => props.device?.device_slot,
  (slot) => {
    bindSlot.value = slot ?? ''
  },
  { immediate: true },
)

watch(
  () => props.roomId,
  () => {
    machines.value = []
    machinesError.value = ''
    machineToken.value = null
    machineRemoveTarget.value = null
    machineFormOpen.value = false
    void loadMachines()
  },
  { immediate: true },
)

function reportError(caught: unknown, fallback: string) {
  if (isForbidden(caught)) {
    toast.error('缺少权限', '当前账号没有设备管理权限（device:manage）。')
    return
  }
  if (caught instanceof ApiError && caught.status === 409) {
    toast.error('槽位冲突', '该设备槽位已被其它房间占用，请更换槽位或先解绑。')
    return
  }
  toast.error('操作失败', errorMessage(caught, fallback))
}

async function bind() {
  const slot = bindSlot.value.trim()
  if (!slot) {
    toast.error('请输入设备槽位', '槽位用于关联设备注册记录，例如 room-501')
    return
  }
  bindBusy.value = true
  try {
    await api.bindRoomDevice(props.roomId, slot)
    toast.success('设备槽位已绑定', slot)
    tokenResult.value = null
    emit('changed')
  } catch (caught) {
    reportError(caught, '绑定设备槽位失败')
  } finally {
    bindBusy.value = false
  }
}

async function confirmUnbind() {
  unbindBusy.value = true
  try {
    await api.bindRoomDevice(props.roomId, null)
    toast.success('设备槽位已解绑', '设备注册记录与在线事件仍会保留')
    tokenResult.value = null
    unbindOpen.value = false
    emit('changed')
  } catch (caught) {
    reportError(caught, '解绑设备失败')
  } finally {
    unbindBusy.value = false
  }
}

async function issueToken() {
  tokenError.value = ''
  const ttl = Number(tokenTtl.value)
  if (!Number.isInteger(ttl) || ttl < 60 || ttl > 86_400) {
    tokenError.value = '有效期需为 60–86400 秒之间的整数'
    return
  }
  tokenBusy.value = true
  try {
    const result = await api.issueDeviceToken(props.roomId, {
      kind: tokenKind.value,
      ttl_secs: ttl,
    })
    tokenResult.value = result
    toast.success('注册 token 已签发', '明文 token 仅显示一次，请立即复制保存')
    emit('changed')
  } catch (caught) {
    if (isForbidden(caught)) {
      tokenError.value = '缺少权限：当前账号没有设备管理权限（device:manage）。'
    } else {
      tokenError.value = errorMessage(caught, '签发注册 token 失败')
    }
  } finally {
    tokenBusy.value = false
  }
}

/** 子机列表：只展示非主设备行；`pending_token` 表示已建槽位但设备尚未注册。 */
async function loadMachines() {
  const roomId = props.roomId
  const request = ++machinesRequest
  if (!roomId) {
    machines.value = []
    machinesError.value = ''
    return
  }
  machinesLoading.value = true
  machinesError.value = ''
  try {
    const rows = await api.listRoomMachines(roomId)
    if (request !== machinesRequest) return
    machines.value = rows
  } catch (caught) {
    if (request !== machinesRequest) return
    machines.value = []
    machinesError.value = isForbidden(caught)
      ? '缺少权限：当前账号没有设备读取权限（device:read），无法查看房间子机。'
      : errorMessage(caught, '加载房间子机列表失败')
  } finally {
    if (request === machinesRequest) machinesLoading.value = false
  }
}

function machineTitle(machine: RoomMachine): string {
  return machine.label?.trim() || machine.device_slot
}

/** 未注册（null status）时给中性提示，避免与 pending_token 的「待注册」重复。 */
function machineStatusMeta(machine: RoomMachine) {
  if (!machine.status) return { label: '未注册', tone: 'slate' as const }
  return deviceStatusMeta(machine.status)
}

function openMachineForm() {
  machineLabel.value = ''
  machineKind.value = 'auto'
  machineTtl.value = '900'
  machineTokenError.value = ''
  machineFormOpen.value = true
}

function closeMachineForm() {
  machineFormOpen.value = false
  machineTokenError.value = ''
}

async function submitMachineToken() {
  machineTokenError.value = ''
  const ttl = Number(machineTtl.value)
  if (!Number.isInteger(ttl) || ttl < 60 || ttl > 86_400) {
    machineTokenError.value = '有效期需为 60–86400 秒之间的整数'
    return
  }
  const label = machineLabel.value.trim()
  if (label.length > 64) {
    machineTokenError.value = '子机标签不能超过 64 个字符'
    return
  }

  machineTokenBusy.value = true
  try {
    const result = await api.issueRoomMachineToken(props.roomId, {
      kind: machineKind.value,
      ttl_secs: ttl,
      ...(label ? { label } : {}),
    })
    machineToken.value = result
    machineFormOpen.value = false
    toast.success('子机注册 token 已签发', '明文 token 仅显示一次，请立即复制保存')
    await loadMachines()
    emit('changed')
  } catch (caught) {
    if (isForbidden(caught)) {
      machineTokenError.value = '缺少权限：当前账号没有设备管理权限（device:manage）。'
    } else {
      machineTokenError.value = errorMessage(caught, '签发子机注册 token 失败')
    }
  } finally {
    machineTokenBusy.value = false
  }
}

async function confirmRemoveMachine() {
  const target = machineRemoveTarget.value
  if (!target) return
  machineRemoveBusy.value = true
  try {
    await api.removeRoomMachine(props.roomId, target.device_slot)
    toast.success('子机已解绑', `槽位 ${target.device_slot} 已从房间移除`)
    machineRemoveTarget.value = null
    await loadMachines()
    emit('changed')
  } catch (caught) {
    reportError(caught, '解绑子机失败')
  } finally {
    machineRemoveBusy.value = false
  }
}
</script>

<template>
  <PageSection title="设备绑定" description="主设备占用一个设备槽位；同一房间还可以绑定多台子机。">
    <div class="space-y-4 px-5 py-4">
      <div v-if="loading" class="space-y-2">
        <div class="h-4 w-2/3 animate-pulse rounded bg-canvas" />
        <div class="h-4 w-1/2 animate-pulse rounded bg-canvas" />
      </div>

      <template v-else-if="device">
        <dl class="grid grid-cols-2 gap-x-4 gap-y-3">
          <div class="min-w-0">
            <dt class="text-[10.5px] text-faint">设备槽位</dt>
            <dd class="mt-0.5 font-mono text-[11.5px] text-ink">
              {{ device.device_slot || '—' }}
            </dd>
          </div>
          <div class="min-w-0">
            <dt class="text-[10.5px] text-faint">序列号</dt>
            <dd class="mt-0.5">
              <CopyText
                :value="device.serial"
                :display="device.serial ? shortId(device.serial, 16) : '未注册'"
                label="序列号"
              />
            </dd>
          </div>
          <div class="min-w-0">
            <dt class="text-[10.5px] text-faint">注册状态</dt>
            <dd class="mt-0.5">
              <StateBadge :label="statusMeta.label" :tone="statusMeta.tone" />
            </dd>
          </div>
          <div class="min-w-0">
            <dt class="text-[10.5px] text-faint">在线状态</dt>
            <dd class="mt-0.5">
              <StateBadge
                :label="onlineMeta.label"
                :tone="onlineMeta.tone"
                :pulse="device.online === 'online'"
              />
            </dd>
          </div>
          <div class="min-w-0">
            <dt class="text-[10.5px] text-faint">最后心跳</dt>
            <dd
              class="mt-0.5 text-[11.5px] text-ink"
              :title="formatFullDateTime(device.last_seen_at)"
            >
              {{ formatRelative(device.last_seen_at) }}
            </dd>
          </div>
          <div class="min-w-0">
            <dt class="text-[10.5px] text-faint">注册 Token</dt>
            <dd class="mt-0.5">
              <StateBadge v-if="device.pending_token" label="待设备注册" tone="amber" pulse />
              <span v-else class="text-[11.5px] text-faint">—</span>
            </dd>
          </div>
        </dl>
      </template>

      <div
        v-else
        class="rounded-md border border-dashed border-line px-3 py-2.5 text-[11.5px] leading-5 text-muted"
      >
        该房间尚未绑定设备。可以在下方输入设备槽位完成绑定，或直接签发注册 token。
      </div>

      <template v-if="canManage">
        <div class="border-t border-line/70 pt-3.5">
          <FormField
            label="绑定 / 迁移设备槽位"
            input-id="room-bind-slot"
            hint="输入已注册设备的槽位；同一槽位会自动从旧房间迁移，留空解绑请点右侧按钮"
          >
            <div class="flex items-center gap-2">
              <input
                id="room-bind-slot"
                v-model="bindSlot"
                class="input font-mono"
                placeholder="room-501"
              />
              <AppButton icon="check" variant="primary" :loading="bindBusy" @click="bind">
                绑定 / 迁移
              </AppButton>
              <AppButton
                :disabled="!hasBoundDevice || bindBusy"
                variant="danger"
                icon="x"
                @click="unbindOpen = true"
              >
                解绑
              </AppButton>
            </div>
          </FormField>
        </div>

        <div class="border-t border-line/70 pt-3.5">
          <div class="grid grid-cols-2 gap-3">
            <FormField label="注册方式" input-id="room-token-kind">
              <select id="room-token-kind" v-model="tokenKind" class="select">
                <option value="auto">自动注册（auto）</option>
                <option value="approval">需要审批（approval）</option>
              </select>
            </FormField>
            <FormField
              label="有效期（秒）"
              input-id="room-token-ttl"
              hint="取值范围 60–86400，默认 900"
            >
              <input
                id="room-token-ttl"
                v-model="tokenTtl"
                type="number"
                min="60"
                max="86400"
                step="1"
                class="input"
              />
            </FormField>
          </div>
          <p v-if="tokenError" class="mt-1.5 text-[11px] text-rose-600">{{ tokenError }}</p>
          <AppButton
            icon="key"
            variant="primary"
            class="mt-3"
            :loading="tokenBusy"
            @click="issueToken"
          >
            签发注册 Token
          </AppButton>
        </div>

        <div
          v-if="tokenResult"
          class="space-y-2.5 rounded-lg border border-amber-200 bg-amber-50 px-3 py-3"
        >
          <div class="flex items-start gap-2 text-[11px] leading-5 text-amber-800">
            <AppIcon name="alert" :size="13" class="mt-0.5 shrink-0" />
            <span>
              token 明文只返回一次，离开或刷新页面后无法再次查看，请立即复制并交给设备端。 槽位
              <span class="font-mono">{{ tokenResult.device_slot }}</span> · 有效期至
              {{ formatFullDateTime(tokenResult.expires_at) }}
            </span>
          </div>
          <div>
            <div class="text-[10.5px] text-amber-700">Token</div>
            <CopyText :value="tokenResult.token" label="注册 Token" />
          </div>
          <div>
            <div class="text-[10.5px] text-amber-700">注册链接</div>
            <CopyText :value="tokenResult.link" label="注册链接" />
          </div>
          <AppButton size="sm" @click="tokenResult = null">我已保存</AppButton>
        </div>
      </template>

      <p v-else class="border-t border-line/70 pt-3 text-[11px] text-faint">
        仅查看模式：绑定设备与签发 token 需要设备管理权限（device:manage）。
      </p>

      <!-- 子机（其他机器）：一个房间可绑定多台，主设备仍由上方槽位管理。 -->
      <div class="border-t border-line/70 pt-3.5">
        <div class="flex items-start justify-between gap-2">
          <div class="min-w-0">
            <h3 class="text-[12.5px] font-medium text-ink">子机（其他机器）</h3>
            <p class="mt-0.5 text-[10.5px] leading-4 text-faint">
              同一房间可以绑定多台子机；主设备仍由上方槽位管理。
            </p>
          </div>
          <AppButton
            v-if="canManage"
            size="sm"
            icon="plus"
            :disabled="machinesLoading"
            @click="openMachineForm"
          >
            添加子机
          </AppButton>
        </div>

        <div v-if="machinesLoading" class="mt-2.5 space-y-2">
          <div class="h-9 animate-pulse rounded-md bg-canvas" />
          <div class="h-9 animate-pulse rounded-md bg-canvas" />
        </div>

        <div
          v-else-if="machinesError"
          role="alert"
          class="mt-2.5 rounded-md border border-amber-200 bg-amber-50 px-3 py-2.5 text-[11.5px] leading-5 text-amber-800"
        >
          <p>{{ machinesError }}</p>
          <AppButton size="sm" variant="ghost" class="mt-1 -ml-1" @click="loadMachines">
            重试
          </AppButton>
        </div>

        <ul
          v-else-if="childMachines.length"
          class="mt-2.5 divide-y divide-line/70 overflow-hidden rounded-md border border-line"
        >
          <li
            v-for="machine in childMachines"
            :key="machine.device_slot"
            class="flex items-start justify-between gap-2 px-3 py-2.5"
          >
            <div class="min-w-0 space-y-1">
              <div class="flex flex-wrap items-center gap-1.5">
                <span class="truncate text-[12px] font-medium text-ink">
                  {{ machineTitle(machine) }}
                </span>
                <StateBadge
                  :label="machineStatusMeta(machine).label"
                  :tone="machineStatusMeta(machine).tone"
                />
                <StateBadge
                  :label="deviceOnlineMeta(machine.online).label"
                  :tone="deviceOnlineMeta(machine.online).tone"
                  :pulse="machine.online === 'online'"
                />
                <StateBadge v-if="machine.pending_token" label="待注册" tone="amber" pulse />
              </div>
              <div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-[10.5px] text-faint">
                <span>
                  槽位 <span class="font-mono text-ink-soft">{{ machine.device_slot }}</span>
                </span>
                <span v-if="machine.serial" class="inline-flex items-center gap-1">
                  序列号
                  <CopyText
                    :value="machine.serial"
                    :display="shortId(machine.serial, 16)"
                    label="序列号"
                  />
                </span>
                <span v-else>序列号 未注册</span>
                <span :title="formatFullDateTime(machine.last_seen_at)">
                  最后心跳 {{ formatRelative(machine.last_seen_at) }}
                </span>
              </div>
            </div>
            <IconButton
              v-if="canManage"
              icon="trash"
              label="解绑子机"
              size="sm"
              @click="machineRemoveTarget = machine"
            />
          </li>
        </ul>

        <div
          v-else
          class="mt-2.5 rounded-md border border-dashed border-line px-3 py-2.5 text-[11.5px] leading-5 text-muted"
        >
          该房间还没有子机。
          <template v-if="canManage">点击右上角「添加子机」签发注册 token。</template>
        </div>

        <div
          v-if="machineToken"
          class="mt-2.5 space-y-2.5 rounded-lg border border-amber-200 bg-amber-50 px-3 py-3"
        >
          <div class="flex items-start gap-2 text-[11px] leading-5 text-amber-800">
            <AppIcon name="alert" :size="13" class="mt-0.5 shrink-0" />
            <span>
              子机 token 明文只返回一次，离开或刷新页面后无法再次查看，请立即复制。 槽位
              <span class="font-mono">{{ machineToken.device_slot }}</span> · 有效期至
              {{ formatFullDateTime(machineToken.expires_at) }}
            </span>
          </div>
          <div>
            <div class="text-[10.5px] text-amber-700">Token</div>
            <CopyText :value="machineToken.token" label="子机注册 Token" />
          </div>
          <div>
            <div class="text-[10.5px] text-amber-700">注册链接</div>
            <CopyText :value="machineToken.link" label="子机注册链接" />
          </div>
          <AppButton size="sm" @click="machineToken = null">我已保存</AppButton>
        </div>
      </div>
    </div>
  </PageSection>

  <ConfirmDialog
    :open="unbindOpen"
    title="解绑设备槽位"
    description="确认解绑该房间的设备槽位吗？"
    detail="设备注册记录与历史在线事件会保留，但房间将不再关联该设备。"
    confirm-label="解绑"
    danger
    :loading="unbindBusy"
    @close="unbindOpen = false"
    @confirm="confirmUnbind"
  />

  <ConfirmDialog
    :open="machineRemoveTarget !== null"
    title="解绑子机"
    :description="machineRemoveDescription"
    detail="仅解除房间与该子机的绑定关系；设备注册记录与已签发证书仍会保留。"
    confirm-label="解绑"
    danger
    :loading="machineRemoveBusy"
    @close="machineRemoveTarget = null"
    @confirm="confirmRemoveMachine"
  />

  <AppModal
    :open="machineFormOpen"
    title="添加子机"
    description="为房间新增一台子机并签发一次性注册 token；槽位由后端自动分配（{房间编码}-m{n}）。"
    size="sm"
    @close="closeMachineForm"
  >
    <div class="space-y-3">
      <FormField
        label="子机标签（可选）"
        input-id="room-machine-label"
        hint="便于在管理端识别，例如「左侧投影机」，最多 64 个字符"
      >
        <input
          id="room-machine-label"
          v-model="machineLabel"
          class="input"
          maxlength="64"
          placeholder="左侧投影机"
        />
      </FormField>

      <div class="grid grid-cols-2 gap-3">
        <FormField label="注册方式" input-id="room-machine-kind">
          <select id="room-machine-kind" v-model="machineKind" class="select">
            <option value="auto">自动注册（auto）</option>
            <option value="approval">需要审批（approval）</option>
          </select>
        </FormField>
        <FormField
          label="有效期（秒）"
          input-id="room-machine-ttl"
          hint="取值范围 60–86400，默认 900"
        >
          <input
            id="room-machine-ttl"
            v-model="machineTtl"
            type="number"
            min="60"
            max="86400"
            step="1"
            class="input"
          />
        </FormField>
      </div>

      <p v-if="machineTokenError" class="text-[11px] text-rose-600">{{ machineTokenError }}</p>
    </div>

    <template #footer>
      <AppButton :disabled="machineTokenBusy" @click="closeMachineForm">取消</AppButton>
      <AppButton
        icon="key"
        variant="primary"
        :loading="machineTokenBusy"
        @click="submitMachineToken"
      >
        签发子机 Token
      </AppButton>
    </template>
  </AppModal>
</template>
