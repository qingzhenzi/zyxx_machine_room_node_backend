//! VM 远程控制（阶段 1.5）的前端纯逻辑：校验、命令参数、结果解析。
//!
//! 与后端 `crates/api/src/api/admin/machines.rs` 的校验规则保持一致：
//! 名称 `[A-Za-z0-9._-]{1,64}`；create 的 vcpus 1..=64 / memory_mib 256..=262144 /
//! disk_gb 1..=2048；base_volume 与 base_path 二选一（前端只做 base_volume 选择）。

import type { ImageChannel, VmCommandOp } from '@/api/schema'

export const VM_OP_LABELS: Record<VmCommandOp, string> = {
  list: '刷新列表',
  start: '启动',
  shutdown: '关机',
  reboot: '重启',
  destroy: '销毁',
  screenshot: '截图',
  create: '新建虚拟机',
  'snapshot.create': '创建快照',
  'snapshot.list': '快照列表',
  'snapshot.restore': '恢复快照',
  'snapshot.delete': '删除快照',
  'disk.info': '磁盘链',
  export: '导出磁盘',
  'image.make': '制作镜像',
}

/** 与服务端一致的名称白名单（防路径注入）。 */
export const VM_NAME_PATTERN = /^[A-Za-z0-9._-]{1,64}$/

export const VM_CREATE_LIMITS = {
  vcpus: { min: 1, max: 64 },
  memoryMiB: { min: 256, max: 262_144 },
  diskGiB: { min: 1, max: 2_048 },
} as const

export interface VmCreateForm {
  name: string
  vcpus: number
  memoryMiB: number
  diskGiB: number
  /** 底包 libvirt 卷名（`<image name>-<version>`）；空 = 空白盘。 */
  baseVolume: string
  start: boolean
  /** PCI 直通地址，逗号分隔（可选，最多 2 个）。 */
  gpu: string
}

export interface VmSnapshotRow {
  tag: string
  createdAt: string | null
  vmSize: string | null
  date: string | null
}

export interface VmDiskEntry {
  path: string
  format: string | null
  virtualSize: number | null
  backingPath: string | null
  backingFormat: string | null
}

export interface VmDiskInfoView {
  disks: VmDiskEntry[]
  chain: string[]
}

export interface VmExportView {
  path: string
  sha256: string | null
  sizeBytes: number | null
}

export function isValidVmName(name: string): boolean {
  return VM_NAME_PATTERN.test(name.trim())
}

/** 返回第一条错误信息；null = 合法。 */
export function validateVmCreateForm(form: VmCreateForm): string | null {
  const name = form.name.trim()
  if (!name) return '请填写虚拟机名称'
  if (!isValidVmName(name)) return '名称只允许字母、数字、点、下划线、短横线（1-64 字符）'
  if (
    !Number.isInteger(form.vcpus) ||
    form.vcpus < VM_CREATE_LIMITS.vcpus.min ||
    form.vcpus > VM_CREATE_LIMITS.vcpus.max
  ) {
    return `vCPU 需在 ${VM_CREATE_LIMITS.vcpus.min}..=${VM_CREATE_LIMITS.vcpus.max} 之间`
  }
  if (
    !Number.isInteger(form.memoryMiB) ||
    form.memoryMiB < VM_CREATE_LIMITS.memoryMiB.min ||
    form.memoryMiB > VM_CREATE_LIMITS.memoryMiB.max
  ) {
    return `内存需在 ${VM_CREATE_LIMITS.memoryMiB.min}..=${VM_CREATE_LIMITS.memoryMiB.max} MiB 之间`
  }
  if (
    !Number.isInteger(form.diskGiB) ||
    form.diskGiB < VM_CREATE_LIMITS.diskGiB.min ||
    form.diskGiB > VM_CREATE_LIMITS.diskGiB.max
  ) {
    return `磁盘需在 ${VM_CREATE_LIMITS.diskGiB.min}..=${VM_CREATE_LIMITS.diskGiB.max} GiB 之间`
  }
  const gpu = parseGpuInput(form.gpu)
  if (gpu.error) return gpu.error
  return null
}

/** 把表单转成 create 的 args（调用前应先 `validateVmCreateForm`）。 */
export function buildCreateArgs(form: VmCreateForm): Record<string, unknown> {
  const args: Record<string, unknown> = {
    name: form.name.trim(),
    vcpus: form.vcpus,
    memory_mib: form.memoryMiB,
    disk_gb: form.diskGiB,
    start: form.start,
  }
  const baseVolume = form.baseVolume.trim()
  if (baseVolume) args.base_volume = baseVolume
  const gpu = parseGpuInput(form.gpu)
  if (gpu.values.length) args.gpu = gpu.values
  return args
}

/** 解析逗号分隔的 GPU 地址；最多 2 个，格式 `dddd:bb:dd.f`。 */
export function parseGpuInput(raw: string): { values: string[]; error: string | null } {
  const values = raw
    .split(',')
    .map((item) => item.trim())
    .filter((item) => item.length > 0)
  if (values.length > 2) return { values, error: 'GPU 最多 2 个' }
  const pci = /^[0-9a-fA-F]{4}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]$/
  for (const value of values) {
    if (!pci.test(value)) {
      return { values, error: `GPU 地址非法（${value}），应为 dddd:bb:dd.f` }
    }
  }
  return { values, error: null }
}

/** 批量下发的槽位尾号（与后端 `sanitize_slot_suffix` 同规则，用于界面预览）。 */
export function slotSuffix(slot: string): string {
  const trimmed = slot.trim()
  const candidate =
    trimmed
      .split(/[-_]/)
      .filter((part) => part.length > 0)
      .pop() ?? trimmed
  const cleaned = candidate.replace(/[^A-Za-z0-9._-]/g, '').replace(/^[._-]+|[._-]+$/g, '')
  return cleaned || '1'
}

export function batchVmNamePreview(prefix: string, slot: string): string {
  let suffix = slotSuffix(slot)
  if (suffix.length > 62) suffix = suffix.slice(0, 62)
  const maxPrefix = Math.max(1, 64 - (suffix.length + 1))
  return `${prefix.trim().slice(0, maxPrefix)}-${suffix}`
}

// ---- 「制作镜像」（image.make：导出 → 分片上传 → register → verify → publish） ----

/** 镜像名（与 qe_domain::validate_image_name 一致）：小写字母/数字/`-_.`，1..=64。 */
export const IMAGE_NAME_PATTERN = /^[a-z0-9._-]{1,64}$/
/** 镜像版本（与 qe_domain::validate_image_version 一致）。 */
export const IMAGE_VERSION_PATTERN = /^[A-Za-z0-9._+-]{1,64}$/
/** 平台 `{os}/{arch}`，两段各 1..=32，字母/数字/`-_.`。 */
export const IMAGE_PLATFORM_PATTERN = /^[A-Za-z0-9._-]{1,32}\/[A-Za-z0-9._-]{1,32}$/

export interface VmMakeImageForm {
  name: string
  version: string
  platform: string
  /** 可选：该 VM 使用的底包卷名（仅写进 manifest annotations 作为来源信息）。 */
  baseVolume: string
  description: string
  /** 发布通道（verify 通过后 publish 用）。 */
  channel: ImageChannel
}

export interface VmMakeImageResult {
  imageId: string
  manifestDigest: string
  totalSize: number | null
  chunks: number | null
}

/** 运行中的 VM 不允许制作镜像（导出要求关机，避免 qcow2 元数据不一致）。 */
export function isVmRunningState(state: string | null | undefined): boolean {
  const normalized = (state ?? '').trim().toLowerCase()
  return ['running', 'paused', 'restarting', 'blocked', 'in shutdown'].includes(normalized)
}

/** 返回第一条错误信息；null = 合法。 */
export function validateMakeImageForm(form: VmMakeImageForm): string | null {
  if (!IMAGE_NAME_PATTERN.test(form.name.trim())) {
    return '镜像名只允许小写字母、数字、点、下划线、短横线（1-64 字符）'
  }
  if (!IMAGE_VERSION_PATTERN.test(form.version.trim())) {
    return '镜像版本只允许字母、数字、点、下划线、短横线、加号（1-64 字符）'
  }
  if (!IMAGE_PLATFORM_PATTERN.test(form.platform.trim())) {
    return '平台格式应为 {os}/{arch}（如 linux/amd64）'
  }
  const baseVolume = form.baseVolume.trim()
  if (baseVolume && !/^[A-Za-z0-9._-]{1,128}$/.test(baseVolume)) {
    return '底包卷名只允许字母、数字、点、下划线、短横线（1-128 字符）'
  }
  if (form.description.trim().length > 255) return '说明不能超过 255 个字符'
  return null
}

/** 构造 `image.make` 的 args（服务端/agent 契约见设计 §8.5）。 */
export function buildMakeImageArgs(vmName: string, form: VmMakeImageForm): Record<string, unknown> {
  const args: Record<string, unknown> = {
    vm_name: vmName.trim(),
    name: form.name.trim(),
    version: form.version.trim(),
    platform: form.platform.trim(),
  }
  const baseVolume = form.baseVolume.trim()
  if (baseVolume) args.base_volume = baseVolume
  const description = form.description.trim()
  if (description) args.description = description
  return args
}

/** 解析 `image.make` 的 result：`{image_id, manifest_digest, total_size?, chunks?}`。 */
export function parseMakeImageResult(data: unknown): VmMakeImageResult | null {
  const root = asObject(data)
  if (!root) return null
  const imageId = asString(root.image_id) ?? asString(root.imageId)
  const manifestDigest = asString(root.manifest_digest) ?? asString(root.manifestDigest)
  if (!imageId || !manifestDigest) return null
  return {
    imageId,
    manifestDigest,
    totalSize: asNumber(root.total_size) ?? asNumber(root.totalSize),
    chunks: asNumber(root.chunks),
  }
}

function asObject(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null
}

function asString(value: unknown): string | null {
  if (typeof value === 'string') {
    const trimmed = value.trim()
    return trimmed || null
  }
  if (typeof value === 'number' && Number.isFinite(value)) return String(value)
  return null
}

function asNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value)
    return Number.isFinite(parsed) ? parsed : null
  }
  return null
}

/** `snapshot.list` 结果：`{snapshots:[{tag, created_at, vm_size, date}]}`。 */
export function parseSnapshotList(data: unknown): VmSnapshotRow[] {
  const root = asObject(data)
  const raw = root && Array.isArray(root.snapshots) ? root.snapshots : []
  const rows: VmSnapshotRow[] = []
  for (const item of raw) {
    const snapshot = asObject(item)
    if (!snapshot) continue
    const tag = asString(snapshot.tag) ?? asString(snapshot.name) ?? asString(snapshot.id)
    if (!tag) continue
    rows.push({
      tag,
      createdAt: asString(snapshot.created_at) ?? asString(snapshot.createdAt),
      vmSize: asString(snapshot.vm_size) ?? asString(snapshot.vmSize),
      date: asString(snapshot.date),
    })
  }
  return rows
}

/** `disk.info` 结果：`{disks:[{path,format,virtual_size,backing_path,backing_format}], chain:[...]}`。 */
export function parseDiskInfo(data: unknown): VmDiskInfoView | null {
  const root = asObject(data)
  if (!root) return null
  const rawDisks = Array.isArray(root.disks) ? root.disks : []
  const disks: VmDiskEntry[] = []
  for (const item of rawDisks) {
    const disk = asObject(item)
    if (!disk) continue
    const path = asString(disk.path) ?? asString(disk.filename)
    if (!path) continue
    disks.push({
      path,
      format: asString(disk.format),
      virtualSize: asNumber(disk.virtual_size) ?? asNumber(disk.virtualSize),
      backingPath: asString(disk.backing_path) ?? asString(disk.backingPath),
      backingFormat: asString(disk.backing_format) ?? asString(disk.backingFormat),
    })
  }
  const rawChain = Array.isArray(root.chain) ? root.chain : []
  const chain: string[] = []
  for (const item of rawChain) {
    const label = asString(item) ?? asString(asObject(item)?.path)
    if (label) chain.push(label)
  }
  if (!disks.length && !chain.length) return null
  return { disks, chain }
}

/** `export` 结果：`{path, sha256, size_bytes}`。 */
export function parseExportResult(data: unknown): VmExportView | null {
  const root = asObject(data)
  if (!root) return null
  const path = asString(root.path) ?? asString(root.file)
  if (!path) return null
  return {
    path,
    sha256: asString(root.sha256),
    sizeBytes: asNumber(root.size_bytes) ?? asNumber(root.sizeBytes),
  }
}

/** 命令终态：pending 之外都算结束。 */
export function isTerminalVmStatus(status: string): boolean {
  return status === 'ok' || status === 'error' || status === 'timeout'
}
