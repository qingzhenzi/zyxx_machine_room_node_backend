<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'

import { api } from '@/api'
import type {
  ImageArtifact,
  RoomStartupConfig,
  RoomStartupProfile,
  RoomStartupTarget,
} from '@/api/schema'
import AppButton from '@/components/ui/AppButton.vue'
import PageSection from '@/components/ui/PageSection.vue'
import StateBadge from '@/components/ui/StateBadge.vue'
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue'
import { useToastStore } from '@/stores/toast'
import { errorMessage } from '@/utils/errors'
import { formatFullDateTime } from '@/utils/format'

/**
 * 房间启动配置（startup provisioning）：
 * - 读取 `GET /admin/rooms/{id}/startup`，编辑后 `PUT` 保存；
 * - agent 收到 `startup.update` 后立即拉取，缺失镜像自动 image.pull，VM 不存在则 create、
 *   关机则 start（默认 60s 周期兜底）；
 * - 只有 `vm:control` 权限可编辑，其余账号只读。
 */

const props = defineProps<{
  roomId: string
  canManage: boolean
}>()

interface VmFormRow {
  name: string
  vcpus: number
  memory_mib: number
  disk_gb: number
  base_volume: string
  base_path: string
  gpu: string
  auto_start: boolean
}

const MAX_IMAGES = 8
const MAX_VMS = 32
const VM_NAME_RE = /^[A-Za-z0-9._-]{1,64}$/
const VOLUME_RE = /^[A-Za-z0-9._-]{1,128}$/
const GPU_RE = /^[0-9a-fA-F]{4}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]$/

const toast = useToastStore()

const loading = ref(false)
const saving = ref(false)
const error = ref('')
const profile = ref<RoomStartupProfile | null>(null)
const vmDiskImages = ref<ImageArtifact[]>([])
const editing = ref(false)

const form = reactive({
  enabled: true,
  autoPull: true,
  autoStartVm: true,
  imageIds: [] as string[],
  vms: [] as VmFormRow[],
})

/** 已发布 vm_disk 镜像 → 底包卷名候选（`<name>-<version>`，导入器落盘规则）。 */
const vmDiskOptions = computed(() =>
  vmDiskImages.value.map((image) => ({
    id: image.id,
    volume: `${image.name}-${image.version}`,
    label: `${image.name}-${image.version}`,
    description: `${image.platform} · ${image.format}`,
  })),
)

const targets = computed<RoomStartupTarget[]>(() => profile.value?.targets ?? [])
const activeTargets = computed(() => targets.value.filter((target) => target.serial))
const savedImages = computed(() => profile.value?.images ?? [])
const savedVms = computed(() => profile.value?.config.vms ?? [])

const statusLabel = computed(() => {
  if (!profile.value) return '未配置'
  if (!profile.value.enabled) return '已停用'
  if (!profile.value.revision) return '未配置'
  return '已启用'
})
const statusTone = computed<'emerald' | 'slate' | 'amber'>(() => {
  if (!profile.value || !profile.value.revision) return 'slate'
  return profile.value.enabled ? 'emerald' : 'amber'
})

function resetForm() {
  const config = profile.value?.config
  form.enabled = profile.value?.enabled ?? true
  form.autoPull = config?.auto_pull ?? true
  form.autoStartVm = config?.auto_start_vm ?? true
  form.imageIds = (config?.images ?? []).map((image) => image.image_id)
  form.vms = (config?.vms ?? []).map((vm) => ({
    name: vm.name,
    vcpus: vm.vcpus,
    memory_mib: vm.memory_mib,
    disk_gb: vm.disk_gb,
    base_volume: vm.base_volume ?? '',
    base_path: vm.base_path ?? '',
    gpu: (vm.gpu ?? []).join(','),
    auto_start: vm.auto_start,
  }))
  editing.value = false
}

async function loadImages() {
  try {
    const page = await api.listImages({ kind: 'vm_disk', status: 'published', page: 1, page_size: 50 })
    vmDiskImages.value = page.items ?? []
  } catch {
    vmDiskImages.value = []
  }
}

async function load() {
  if (!props.roomId) return
  loading.value = true
  error.value = ''
  try {
    const startup = await api.getRoomStartup(props.roomId)
    profile.value = startup
    resetForm()
  } catch (caught) {
    error.value = errorMessage(caught, '加载房间启动配置失败')
    profile.value = null
  } finally {
    loading.value = false
  }
  void loadImages()
}

function startEdit() {
  if (!props.canManage) return
  resetForm()
  error.value = ''
  editing.value = true
  if (!vmDiskImages.value.length) void loadImages()
}

function cancelEdit() {
  error.value = ''
  resetForm()
}

function toggleImage(imageId: string) {
  const index = form.imageIds.indexOf(imageId)
  if (index >= 0) {
    form.imageIds.splice(index, 1)
    return
  }
  if (form.imageIds.length >= MAX_IMAGES) {
    toast.error('镜像数量超限', `启动配置最多 ${MAX_IMAGES} 个镜像`)
    return
  }
  form.imageIds.push(imageId)
}

function addVm() {
  if (form.vms.length >= MAX_VMS) {
    toast.error('VM 数量超限', `启动配置最多 ${MAX_VMS} 台 VM`)
    return
  }
  form.vms.push({
    name: '',
    vcpus: 2,
    memory_mib: 2048,
    disk_gb: 20,
    base_volume: '',
    base_path: '',
    gpu: '',
    auto_start: true,
  })
}

function removeVm(index: number) {
  form.vms.splice(index, 1)
}

function parseGpu(raw: string): string[] {
  return raw
    .split(',')
    .map((item) => item.trim())
    .filter(Boolean)
}

/** 返回第一条校验错误；无错误返回空串。 */
function validate(): string {
  if (form.imageIds.length > MAX_IMAGES) return `镜像最多 ${MAX_IMAGES} 个`
  if (form.vms.length > MAX_VMS) return `VM 最多 ${MAX_VMS} 台`
  const seen = new Set<string>()
  for (const [index, vm] of form.vms.entries()) {
    const label = `第 ${index + 1} 台 VM`
    const name = vm.name.trim()
    if (!VM_NAME_RE.test(name)) {
      return `${label}：名称只允许字母、数字、点、下划线、短横线（1-64 字符）`
    }
    if (seen.has(name)) return `${label}：名称 ${name} 重复`
    seen.add(name)
    if (!Number.isInteger(vm.vcpus) || vm.vcpus < 1 || vm.vcpus > 64) {
      return `${label}：vCPU 必须是 1-64 的整数`
    }
    if (!Number.isInteger(vm.memory_mib) || vm.memory_mib < 256 || vm.memory_mib > 262144) {
      return `${label}：内存必须在 256-262144 MiB`
    }
    if (!Number.isInteger(vm.disk_gb) || vm.disk_gb < 1 || vm.disk_gb > 2048) {
      return `${label}：磁盘必须在 1-2048 GiB`
    }
    const volume = vm.base_volume.trim()
    const path = vm.base_path.trim()
    if (volume && path) return `${label}：base_volume 与 base_path 只能二选一`
    if (volume && !VOLUME_RE.test(volume)) return `${label}：base_volume 格式非法`
    if (path && !path.startsWith('/')) return `${label}：base_path 必须是绝对路径`
    const gpu = parseGpu(vm.gpu)
    if (gpu.length > 2) return `${label}：GPU 直通最多 2 个`
    if (gpu.some((item) => !GPU_RE.test(item))) return `${label}：GPU 地址格式应为 dddd:bb:dd.f`
  }
  return ''
}

function buildConfig(): RoomStartupConfig {
  return {
    auto_pull: form.autoPull,
    auto_start_vm: form.autoStartVm,
    images: form.imageIds.map((imageId) => ({ image_id: imageId })),
    vms: form.vms.map((vm) => {
      const gpu = parseGpu(vm.gpu)
      return {
        name: vm.name.trim(),
        vcpus: vm.vcpus,
        memory_mib: vm.memory_mib,
        disk_gb: vm.disk_gb,
        base_volume: vm.base_volume.trim() || null,
        base_path: vm.base_path.trim() || null,
        gpu,
        auto_start: vm.auto_start,
      }
    }),
  }
}

async function save() {
  if (!props.canManage || saving.value) return
  const validation = validate()
  error.value = validation
  if (validation) return
  saving.value = true
  try {
    const saved = await api.saveRoomStartup(props.roomId, {
      enabled: form.enabled,
      config: buildConfig(),
    })
    profile.value = saved
    resetForm()
    toast.success(
      `启动配置已保存（revision ${saved.revision}）`,
      saved.enabled
        ? 'agent 收到 startup.update 后立即拉起，最长 60s 周期兜底'
        : '已停用：agent 下次 reconcile 只记录配置，不做任何操作',
    )
  } catch (caught) {
    error.value = errorMessage(caught, '保存房间启动配置失败')
  } finally {
    saving.value = false
  }
}

watch(() => props.roomId, () => void load())
onMounted(() => void load())
</script>

<template>
  <PageSection
    title="启动配置"
    description="设备上线后自动拉取镜像、创建并启动虚拟机（agent 收到 startup.update 立即执行，默认 60s 周期兜底）"
  >
    <div class="px-5 py-4">
      <div v-if="error" role="alert" class="mb-3 rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-[11.5px] text-rose-700">
        {{ error }}
      </div>

      <div v-if="loading && !profile" class="text-[11.5px] text-muted">正在加载启动配置…</div>

      <template v-else-if="!editing">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div class="flex flex-wrap items-center gap-2">
            <StateBadge :label="statusLabel" :tone="statusTone" />
            <span class="text-[11px] text-faint">revision {{ profile?.revision ?? 0 }}</span>
            <span v-if="profile?.updated_at" class="text-[11px] text-faint">
              · 更新于 {{ formatFullDateTime(profile.updated_at) }}
            </span>
            <span class="text-[11px] text-faint">
              · 生效槽位
              <template v-if="targets.length">
                {{ targets.map((target) => target.device_slot).join('、') }}
                （{{ activeTargets.length }}/{{ targets.length }} 有存活证书）
              </template>
              <template v-else>未绑定</template>
            </span>
          </div>
          <AppButton v-if="canManage" icon="pencil" size="sm" @click="startEdit">编辑配置</AppButton>
        </div>

        <div class="mt-3 grid grid-cols-2 gap-4">
          <div>
            <div class="text-[10.5px] font-medium text-faint">自动拉取镜像 / 自动启动 VM</div>
            <p class="mt-1 text-[11.5px] text-ink-soft">
              {{ (profile?.config.auto_pull ?? true) ? '开' : '关' }} /
              {{ (profile?.config.auto_start_vm ?? true) ? '开' : '关' }}
            </p>
            <div class="mt-3 text-[10.5px] font-medium text-faint">镜像（{{ savedImages.length }}）</div>
            <ul v-if="savedImages.length" class="mt-1.5 divide-y divide-line/70">
              <li
                v-for="image in savedImages"
                :key="image.image_id"
                class="flex items-center justify-between gap-2 py-1.5 text-[11.5px]"
              >
                <span class="min-w-0">
                  <span class="text-ink">{{ image.name }}-{{ image.version }}</span>
                  <span class="ml-1.5 text-[10.5px] text-faint">{{ image.kind }}</span>
                </span>
                <StateBadge
                  :label="image.assigned ? '已分配' : '待分配'"
                  :tone="image.assigned ? 'emerald' : 'amber'"
                />
              </li>
            </ul>
            <p v-else class="mt-1.5 text-[11px] text-faint">未配置镜像（agent 不会自动拉取）</p>
          </div>

          <div>
            <div class="text-[10.5px] font-medium text-faint">虚拟机（{{ savedVms.length }}）</div>
            <div v-if="savedVms.length" class="mt-1.5 space-y-1.5">
              <div
                v-for="vm in savedVms"
                :key="vm.name"
                class="rounded-lg border border-line/70 bg-canvas/40 px-3 py-2 text-[11.5px]"
              >
                <div class="flex items-center justify-between gap-2">
                  <span class="font-medium text-ink">{{ vm.name }}</span>
                  <StateBadge :label="vm.auto_start ? '自动启动' : '仅创建'" :tone="vm.auto_start ? 'emerald' : 'slate'" />
                </div>
                <p class="mt-0.5 text-[10.5px] text-faint">
                  {{ vm.vcpus }} vCPU · {{ vm.memory_mib }} MiB · {{ vm.disk_gb }} GiB ·
                  底包 {{ vm.base_volume || vm.base_path || '由镜像推导' }}
                </p>
              </div>
            </div>
            <p v-else class="mt-1.5 text-[11px] text-faint">未配置 VM（agent 不会自动创建）</p>
          </div>
        </div>

        <div
          v-if="!profile?.revision"
          class="mt-3 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-dashed border-line bg-canvas/30 px-4 py-3"
        >
          <p class="max-w-[640px] text-[11.5px] leading-5 text-muted">
            尚未配置：保存后本房间设备连上服务端即会自动准备环境（镜像先分配再预签名下发，VM 使用只读底包 + qcow2 差分盘）。
          </p>
          <AppButton v-if="canManage" icon="plus" @click="startEdit">配置镜像与 VM</AppButton>
        </div>
      </template>

      <template v-else>
        <div class="space-y-4">
          <div class="flex flex-wrap items-center gap-4">
            <label class="flex items-center gap-2 text-[11.5px] text-ink-soft">
              <ToggleSwitch v-model="form.enabled" />
              启用启动配置
            </label>
            <label class="flex items-center gap-2 text-[11.5px] text-ink-soft">
              <ToggleSwitch v-model="form.autoPull" :disabled="!form.enabled" />
              自动拉取缺失镜像
            </label>
            <label class="flex items-center gap-2 text-[11.5px] text-ink-soft">
              <ToggleSwitch v-model="form.autoStartVm" :disabled="!form.enabled" />
              自动创建并启动 VM
            </label>
          </div>

          <div>
            <div class="flex items-center justify-between">
              <div class="text-[10.5px] font-medium text-faint">
                镜像（已发布 vm_disk，最多 {{ MAX_IMAGES }} 个；保存时自动分配到本房间用于预签名下载）
              </div>
              <span class="text-[10.5px] text-faint">已选 {{ form.imageIds.length }}</span>
            </div>
            <div v-if="vmDiskOptions.length" class="mt-1.5 grid grid-cols-2 gap-1.5">
              <label
                v-for="option in vmDiskOptions"
                :key="option.id"
                class="flex cursor-pointer items-center gap-2 rounded-lg border border-line/70 px-3 py-1.5 text-[11.5px] transition-colors hover:border-line-strong"
                :class="form.imageIds.includes(option.id) ? 'border-accent/40 bg-accent/5' : ''"
              >
                <input
                  type="checkbox"
                  class="h-3.5 w-3.5 rounded border-line text-accent focus:ring-accent/25"
                  :checked="form.imageIds.includes(option.id)"
                  @change="toggleImage(option.id)"
                />
                <span class="min-w-0">
                  <span class="block truncate text-ink">{{ option.label }}</span>
                  <span class="block truncate text-[10px] text-faint">{{ option.description }}</span>
                </span>
              </label>
            </div>
            <p v-else class="mt-1.5 text-[11px] text-faint">
              暂无已发布 vm_disk 镜像：先到「镜像」页发布 VM 磁盘镜像。
            </p>
          </div>

          <div>
            <div class="flex items-center justify-between">
              <div class="text-[10.5px] font-medium text-faint">虚拟机（最多 {{ MAX_VMS }} 台）</div>
              <AppButton size="sm" icon="plus" @click="addVm">添加 VM</AppButton>
            </div>
            <p v-if="!form.vms.length" class="mt-1.5 text-[11px] text-faint">
              尚未添加 VM。VM 使用只读底包 + qcow2 差分盘；底包不填时按唯一 vm_disk 镜像推导
              &lt;name&gt;-&lt;version&gt;。
            </p>
            <div v-for="(vm, index) in form.vms" :key="index" class="mt-2 rounded-lg border border-line/70 bg-canvas/30 p-3">
              <div class="flex items-center justify-between">
                <span class="text-[11px] font-medium text-ink-soft">VM {{ index + 1 }}</span>
                <AppButton size="sm" variant="danger" icon="trash" @click="removeVm(index)">移除</AppButton>
              </div>
              <div class="mt-2 grid grid-cols-6 gap-2">
                <label class="col-span-2 flex flex-col gap-1 text-[10.5px] text-muted">
                  VM 名称
                  <input
                    v-model="vm.name"
                    placeholder="lab-104"
                    class="h-8 rounded-md border border-line bg-surface px-2.5 text-[12px] text-ink outline-none focus:border-accent"
                  />
                </label>
                <label class="flex flex-col gap-1 text-[10.5px] text-muted">
                  vCPU
                  <input
                    v-model.number="vm.vcpus"
                    type="number"
                    min="1"
                    max="64"
                    class="h-8 rounded-md border border-line bg-surface px-2.5 text-[12px] text-ink outline-none focus:border-accent"
                  />
                </label>
                <label class="flex flex-col gap-1 text-[10.5px] text-muted">
                  内存 MiB
                  <input
                    v-model.number="vm.memory_mib"
                    type="number"
                    min="256"
                    max="262144"
                    class="h-8 rounded-md border border-line bg-surface px-2.5 text-[12px] text-ink outline-none focus:border-accent"
                  />
                </label>
                <label class="flex flex-col gap-1 text-[10.5px] text-muted">
                  磁盘 GiB
                  <input
                    v-model.number="vm.disk_gb"
                    type="number"
                    min="1"
                    max="2048"
                    class="h-8 rounded-md border border-line bg-surface px-2.5 text-[12px] text-ink outline-none focus:border-accent"
                  />
                </label>
                <label class="flex items-end gap-2 pb-1.5 text-[11px] text-ink-soft">
                  <ToggleSwitch v-model="vm.auto_start" />
                  自动启动
                </label>
              </div>
              <div class="mt-2 grid grid-cols-3 gap-2">
                <label class="flex flex-col gap-1 text-[10.5px] text-muted">
                  底包卷名（base_volume）
                  <input
                    v-model="vm.base_volume"
                    list="startup-base-volumes"
                    placeholder="real-debian-13-0.1.1（留空=按镜像推导）"
                    class="h-8 rounded-md border border-line bg-surface px-2.5 text-[12px] text-ink outline-none focus:border-accent"
                  />
                </label>
                <label class="flex flex-col gap-1 text-[10.5px] text-muted">
                  或底包绝对路径（base_path）
                  <input
                    v-model="vm.base_path"
                    placeholder="/var/lib/libvirt/images/base.qcow2"
                    class="h-8 rounded-md border border-line bg-surface px-2.5 text-[12px] text-ink outline-none focus:border-accent"
                  />
                </label>
                <label class="flex flex-col gap-1 text-[10.5px] text-muted">
                  GPU 直通（可选，逗号分隔 ≤2）
                  <input
                    v-model="vm.gpu"
                    placeholder="0000:01:00.0"
                    class="h-8 rounded-md border border-line bg-surface px-2.5 text-[12px] text-ink outline-none focus:border-accent"
                  />
                </label>
              </div>
            </div>
            <datalist id="startup-base-volumes">
              <option v-for="option in vmDiskOptions" :key="option.id" :value="option.volume">
                {{ option.description }}
              </option>
            </datalist>
          </div>

          <div class="flex items-center justify-end gap-2 border-t border-line/70 pt-3">
            <AppButton :disabled="saving" @click="cancelEdit">取消</AppButton>
            <AppButton variant="primary" icon="check" :loading="saving" @click="save">保存启动配置</AppButton>
          </div>
        </div>
      </template>
    </div>
  </PageSection>
</template>
