import type {
  ArtifactStatus,
  Compression,
  ImageChannel,
  ImageFormat,
  ImageKind,
} from '@/api/schema'
import type { BadgeTone } from '@/components/ui/StateBadge.vue'

export const IMAGE_KIND_LABELS: Record<ImageKind, string> = {
  docker: 'Docker',
  oci: 'OCI',
  lxc_rootfs: 'LXC 根文件系统',
  vm_disk: 'VM 磁盘',
}

export const IMAGE_KIND_OPTIONS: { value: ImageKind; label: string }[] = [
  { value: 'docker', label: 'Docker' },
  { value: 'oci', label: 'OCI' },
  { value: 'lxc_rootfs', label: 'LXC 根文件系统' },
  { value: 'vm_disk', label: 'VM 磁盘' },
]

export const IMAGE_FORMAT_LABELS: Record<ImageFormat, string> = {
  docker_archive: 'docker archive',
  oci_layout: 'OCI layout',
  tar: 'tar',
  qcow2: 'qcow2',
  raw: 'raw',
}

export const IMAGE_FORMAT_OPTIONS: { value: ImageFormat; label: string }[] = [
  { value: 'docker_archive', label: 'docker archive' },
  { value: 'oci_layout', label: 'OCI layout' },
  { value: 'tar', label: 'tar' },
  { value: 'qcow2', label: 'qcow2' },
  { value: 'raw', label: 'raw' },
]

/** kind 与 format 的合法组合（后端会校验，前端切换时给默认值）。 */
export const DEFAULT_FORMAT_BY_KIND: Record<ImageKind, ImageFormat> = {
  docker: 'docker_archive',
  oci: 'oci_layout',
  lxc_rootfs: 'tar',
  vm_disk: 'qcow2',
}

export const COMPRESSION_LABELS: Record<Compression, string> = {
  none: '无压缩',
  zstd: 'zstd',
  gzip: 'gzip',
}

export const ARTIFACT_STATUS_META: Record<ArtifactStatus, { label: string; tone: BadgeTone }> = {
  uploaded: { label: '已上传', tone: 'slate' },
  manifest_ready: { label: 'manifest 就绪', tone: 'sky' },
  verifying: { label: '验证中', tone: 'amber' },
  verified: { label: '已验证', tone: 'emerald' },
  published: { label: '已发布', tone: 'violet' },
  deprecated: { label: '已废弃', tone: 'slate' },
  revoked: { label: '已吊销', tone: 'rose' },
}

export const ARTIFACT_STATUS_OPTIONS = (Object.keys(ARTIFACT_STATUS_META) as ArtifactStatus[]).map(
  (value) => ({ value, label: ARTIFACT_STATUS_META[value].label }),
)

export const IMAGE_CHANNEL_META: Record<ImageChannel, { label: string; tone: BadgeTone }> = {
  dev: { label: 'dev', tone: 'sky' },
  test: { label: 'test', tone: 'amber' },
  prod: { label: 'prod', tone: 'emerald' },
}

export const IMAGE_CHANNEL_OPTIONS: { value: ImageChannel; label: string }[] = [
  { value: 'dev', label: 'dev · 开发' },
  { value: 'test', label: 'test · 测试' },
  { value: 'prod', label: 'prod · 生产' },
]

const FALLBACK_META = { label: '—', tone: 'slate' as BadgeTone }

export function imageKindLabel(value: ImageKind | null | undefined): string {
  return value ? IMAGE_KIND_LABELS[value] : '—'
}

export function imageFormatLabel(value: ImageFormat | null | undefined): string {
  return value ? IMAGE_FORMAT_LABELS[value] : '—'
}

export function compressionLabel(value: Compression | null | undefined): string {
  return value ? COMPRESSION_LABELS[value] : '—'
}

export function artifactStatusMeta(value: ArtifactStatus | null | undefined) {
  return value ? ARTIFACT_STATUS_META[value] : FALLBACK_META
}

export function channelMeta(value: ImageChannel | null | undefined) {
  return value ? IMAGE_CHANNEL_META[value] : FALLBACK_META
}
