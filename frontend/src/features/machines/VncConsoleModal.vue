<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, shallowRef, watch } from 'vue'

import { api } from '@/api'
import type { VncSessionReply } from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import AppModal from '@/components/ui/AppModal.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatRelative } from '@/utils/format'

import RFB from '@novnc/novnc'
import {
  buildVncWsUrl,
  isVncBusyStatus,
  vncConnectionMeta,
  type VncConnectionStatus,
} from './vncSession'

const props = defineProps<{
  /** 机器设备序列号（后端用来定位 device-pki 隧道）。 */
  serial: string
  /** libvirt 虚拟机名。 */
  vm: string
  /** 弹窗开关；打开时申请会话并连接。 */
  open: boolean
}>()

const emit = defineEmits<{
  close: []
}>()

const toast = useToastStore()

/** noVNC 的 RFB 接收「容器元素」并在内部创建 canvas；这里放一个撑满弹窗的 div。 */
const screenRef = ref<HTMLElement | null>(null)
const status = ref<VncConnectionStatus>('idle')
const error = ref('')
const session = ref<VncSessionReply | null>(null)
const rfb = shallowRef<RFB | null>(null)

/** 用户主动关闭（而非服务端/网络断开）时不弹“已断开”提示。 */
let closedByUser = false
/** 自增令牌：忽略上一次打开的异步响应（关闭后重新打开时会有并发）。 */
let runId = 0

const meta = computed(() => vncConnectionMeta(status.value))
const busy = computed(() => isVncBusyStatus(status.value))
const connected = computed(() => status.value === 'connected')
const showPlaceholder = computed(
  () =>
    status.value === 'requesting' ||
    status.value === 'connecting' ||
    status.value === 'idle' ||
    status.value === 'error',
)

function teardown() {
  const instance = rfb.value
  rfb.value = null
  if (!instance) return
  try {
    instance.disconnect()
  } catch {
    // RFB 已断开时 disconnect 可能重复调用，忽略即可。
  }
}

/** 关闭弹窗/组件卸载：断开 WS 并清空本地会话。 */
function stop() {
  closedByUser = true
  runId += 1
  teardown()
  status.value = 'idle'
  error.value = ''
  session.value = null
}

function bindRfb(instance: RFB, reply: VncSessionReply, current: number) {
  instance.addEventListener('connect', () => {
    if (current !== runId) return
    status.value = 'connected'
    error.value = ''
    // 键盘事件直接交给 noVNC，避免第一次按键被弹窗吞掉。
    instance.focus()
  })

  instance.addEventListener('disconnect', (event: Event) => {
    if (current !== runId) return
    const detail = (event as CustomEvent<{ clean?: boolean }>).detail
    rfb.value = null
    status.value = 'closed'
    if (!closedByUser && props.open) {
      toast.info('控制台连接已断开', detail?.clean === false ? '服务器或网络中断' : '连接已关闭')
    }
  })

  instance.addEventListener('securityfailure', (event: Event) => {
    if (current !== runId) return
    const detail = (event as CustomEvent<{ status?: number; reason?: string }>).detail
    status.value = 'error'
    error.value = detail?.reason || `VNC 安全协商失败（status=${detail?.status ?? '未知'}）`
    toast.error('控制台安全协商失败', error.value)
  })

  instance.addEventListener('credentialsrequired', () => {
    if (current !== runId) return
    // 构造时已带 password；这里再兜底一次，兼容后端延迟下发密码的实现。
    if (reply.password) {
      instance.credentials = { password: reply.password }
    } else {
      toast.error('控制台需要密码', 'VNC 会话没有返回一次性密码，请重新打开控制台')
    }
  })

  instance.addEventListener('error', () => {
    if (current !== runId) return
    toast.error('控制台连接异常', '请断开后重试，或检查机器 agent 与 libvirt VNC 状态')
  })
}

async function start() {
  if (!props.serial || !props.vm) return

  const current = ++runId
  closedByUser = false
  teardown()
  session.value = null
  error.value = ''
  status.value = 'requesting'

  try {
    // 1) 换取一次性 VNC 密码 + WS 票据（vm:control）。
    const reply = await api.createVncSession(props.serial, { vm: props.vm })
    if (current !== runId || !props.open) return
    session.value = reply

    // 2) 等弹窗内容渲染完成，再把容器交给 noVNC。
    status.value = 'connecting'
    await nextTick()
    if (current !== runId || !props.open) return

    const target = screenRef.value
    if (!target) throw new Error('控制台画布未挂载')

    const instance = new RFB(
      target,
      buildVncWsUrl({
        serial: props.serial,
        sessionId: reply.session_id,
        ticket: reply.ticket,
        wsUrl: reply.ws_url,
      }),
      {
        credentials: { password: reply.password ?? '' },
        scaleViewport: true,
        clipViewport: false,
        viewOnly: false,
        showDotCursor: true,
        background: '#0b0f1a',
      },
    )
    rfb.value = instance
    bindRfb(instance, reply, current)
  } catch (caught) {
    if (current !== runId) return
    status.value = 'error'
    error.value = errorMessage(caught, '打开控制台失败')
    toast.error('打开控制台失败', error.value)
    teardown()
  }
}

function reconnect() {
  void start()
}

function disconnectByUser() {
  closedByUser = true
  teardown()
  status.value = 'closed'
}

watch(
  () => props.open,
  (open) => {
    if (open) void start()
    else stop()
  },
  { immediate: true },
)

/** 切换机器/虚拟机时重新申请会话。 */
watch(
  () => [props.serial, props.vm],
  () => {
    if (props.open) void start()
  },
)

onBeforeUnmount(() => stop())
</script>

<template>
  <AppModal
    :open="open"
    title="虚拟机控制台"
    :description="`${vm} · noVNC 实时画面（WebSocket）；键鼠会直接注入虚拟机`"
    size="xl"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-3">
      <div class="flex flex-wrap items-center gap-2">
        <StateBadge :label="meta.label" :tone="meta.tone" :pulse="busy" />
        <span v-if="session" class="font-mono text-[10.5px] text-faint">
          session {{ session.session_id.slice(0, 8) }}
        </span>
        <span v-if="session?.expires_at" class="text-[10.5px] text-faint">
          票据有效至 {{ formatRelative(session.expires_at) }}
        </span>
        <span class="flex-1" />
        <span class="text-[10.5px] text-faint">关闭弹窗会自动断开并释放会话</span>
      </div>

      <div
        class="relative flex h-[56vh] min-h-[320px] w-full items-center justify-center overflow-hidden rounded-md border border-line bg-[#0b0f1a]"
      >
        <div ref="screenRef" class="h-full w-full" />
        <p
          v-if="showPlaceholder"
          class="pointer-events-none absolute inset-0 flex items-center justify-center px-6 text-center text-[12px] text-slate-300"
        >
          <span v-if="status === 'requesting'">正在申请一次性 VNC 会话…</span>
          <span v-else-if="status === 'connecting'">正在连接控制节点…</span>
          <span v-else-if="status === 'error'" class="text-rose-300">{{ error }}</span>
          <span v-else>尚未连接</span>
        </p>
      </div>

      <p
        v-if="error && status !== 'error'"
        role="alert"
        class="rounded-md border border-rose-200 bg-rose-50 px-3 py-2 text-[11px] leading-5 text-rose-700"
      >
        {{ error }}
      </p>
    </div>

    <template #footer>
      <AppButton :disabled="busy" @click="emit('close')">关闭</AppButton>
      <AppButton v-if="connected" variant="danger" @click="disconnectByUser">断开</AppButton>
      <AppButton v-else variant="primary" :loading="busy" @click="reconnect">
        {{ status === 'idle' ? '连接' : '重新连接' }}
      </AppButton>
    </template>
  </AppModal>
</template>
