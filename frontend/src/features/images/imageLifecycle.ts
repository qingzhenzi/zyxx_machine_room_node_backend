//! 镜像生命周期（下架 / 删除）的纯逻辑：按钮条件、确认文案、错误归类。
//!
//! 后端契约（`crates/api/src/api/admin/images.rs`）：
//! - `POST /admin/images/{id}/deprecate` → 200 + ImageArtifact；published → deprecated，
//!   deprecated 幂等；其它状态 409。
//! - `DELETE /admin/images/{id}` → 204；published 返回 409（需先下架）；不存在 404。
//! - 两个接口都需要 `image:manage`。

import type { ArtifactStatus, ImageArtifact } from '@/api/schema'

import { artifactStatusMeta } from './labels'

export type ImageDeleteErrorKind = 'conflict' | 'not_found' | 'forbidden' | 'other'

export interface ImageDeleteErrorMeta {
  kind: ImageDeleteErrorKind
  title: string
  description: string
  /** 404（资源已不存在）时页面应刷新列表/详情。 */
  shouldRefresh: boolean
}

export interface ImageDeprecateErrorMeta {
  title: string
  description: string
  shouldRefresh: boolean
}

/** 从 ApiError（或任何带 status 的对象）读取 HTTP 状态码。 */
export function imageErrorStatus(error: unknown): number | undefined {
  if (error && typeof error === 'object' && 'status' in error) {
    const value = (error as { status?: unknown }).status
    return typeof value === 'number' ? value : undefined
  }
  return undefined
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : ''
}

/** 只有已发布（published）的镜像需要/可以下架。 */
export function canDeprecateImage(status: ArtifactStatus | null | undefined): boolean {
  return status === 'published'
}

/** 删除确认框正文（危险文案）：说明会清理 S3 对象且不可恢复。 */
export function deleteConfirmDetail(
  image: Pick<ImageArtifact, 'name' | 'version' | 'status'>,
): string {
  const lines = [
    `镜像：${image.name}:${image.version}（${artifactStatusMeta(image.status).label}）`,
    '删除会移除该镜像的 manifest / 分片 / 验证与通道登记；',
    '不再被其它镜像引用的 S3 对象（分片、manifest、签名）会被一并清理，不可恢复。',
  ]
  if (image.status === 'published') {
    lines.push('当前镜像仍处于已发布状态：后端会拒绝删除（409），请先下架后再删除。')
  }
  return lines.join('\n')
}

/** 下架确认框正文。 */
export function deprecateConfirmDetail(image: Pick<ImageArtifact, 'name' | 'version'>): string {
  return [
    `镜像：${image.name}:${image.version}`,
    '下架后状态变为 deprecated，通道指针与房间分配将不再使用该镜像；',
    '该操作不会删除镜像与 S3 对象，之后仍可查看/删除。',
  ].join('\n')
}

/** 删除失败时的 toast 文案；409 引导先下架、404 提示刷新。 */
export function imageDeleteErrorMeta(error: unknown): ImageDeleteErrorMeta {
  const status = imageErrorStatus(error)
  const message = errorText(error)

  if (status === 409) {
    return {
      kind: 'conflict',
      title: '镜像已发布，无法删除',
      description: message || '请先下架（deprecate）后再删除。',
      shouldRefresh: false,
    }
  }
  if (status === 404) {
    return {
      kind: 'not_found',
      title: '镜像已不存在',
      description: '该镜像可能已被其它操作删除，正在刷新最新数据。',
      shouldRefresh: true,
    }
  }
  if (status === 403) {
    return {
      kind: 'forbidden',
      title: '缺少 image:manage 权限',
      description: '当前账号无权删除镜像（需要 image:manage 权限），请联系管理员。',
      shouldRefresh: false,
    }
  }
  return {
    kind: 'other',
    title: '删除镜像失败',
    description: message || '请稍后重试',
    shouldRefresh: false,
  }
}

/** 下架失败时的 toast 文案。 */
export function imageDeprecateErrorMeta(error: unknown): ImageDeprecateErrorMeta {
  const status = imageErrorStatus(error)
  const message = errorText(error)

  if (status === 409) {
    return {
      title: '无法下架该镜像',
      description: message || '仅已发布（published）的镜像可以下架。',
      shouldRefresh: false,
    }
  }
  if (status === 404) {
    return {
      title: '镜像已不存在',
      description: '该镜像可能已被删除，正在刷新最新数据。',
      shouldRefresh: true,
    }
  }
  if (status === 403) {
    return {
      title: '缺少 image:manage 权限',
      description: '当前账号无权下架镜像（需要 image:manage 权限），请联系管理员。',
      shouldRefresh: false,
    }
  }
  return {
    title: '下架镜像失败',
    description: message || '请稍后重试',
    shouldRefresh: false,
  }
}
