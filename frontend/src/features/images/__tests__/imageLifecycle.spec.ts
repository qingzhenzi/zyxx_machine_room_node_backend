import { describe, expect, it } from 'vitest'

import {
  canDeprecateImage,
  deleteConfirmDetail,
  deprecateConfirmDetail,
  imageDeleteErrorMeta,
  imageDeprecateErrorMeta,
  imageErrorStatus,
} from '../imageLifecycle'

describe('镜像下架/删除判定', () => {
  it('只有 published 的镜像可以下架', () => {
    expect(canDeprecateImage('published')).toBe(true)
    expect(canDeprecateImage('deprecated')).toBe(false)
    expect(canDeprecateImage('verified')).toBe(false)
    expect(canDeprecateImage(null)).toBe(false)
  })

  it('imageErrorStatus 从 ApiError 结构读取 HTTP 状态', () => {
    expect(imageErrorStatus({ status: 409 })).toBe(409)
    expect(imageErrorStatus({ status: '404' })).toBeUndefined()
    expect(imageErrorStatus(new Error('boom'))).toBeUndefined()
    expect(imageErrorStatus(null)).toBeUndefined()
  })
})

describe('确认弹窗危险文案', () => {
  it('删除文案说明 S3 对象清理且不可恢复；published 额外提示先下架', () => {
    const published = deleteConfirmDetail({
      name: 'ubuntu',
      version: '22.04',
      status: 'published',
    })
    expect(published).toContain('ubuntu:22.04')
    expect(published).toContain('S3 对象')
    expect(published).toContain('不可恢复')
    expect(published).toContain('先下架')

    const deprecated = deleteConfirmDetail({
      name: 'ubuntu',
      version: '22.04',
      status: 'deprecated',
    })
    expect(deprecated).toContain('不可恢复')
    expect(deprecated).not.toContain('先下架')
  })

  it('下架文案说明状态变为 deprecated 且不删除对象', () => {
    const detail = deprecateConfirmDetail({ name: 'lab', version: '1.0.0' })
    expect(detail).toContain('lab:1.0.0')
    expect(detail).toContain('deprecated')
    expect(detail).toContain('不会删除')
  })
})

describe('错误归类', () => {
  it('删除 409 → 引导先下架，不刷新', () => {
    const meta = imageDeleteErrorMeta({ status: 409 })
    expect(meta.kind).toBe('conflict')
    expect(meta.title).toContain('已发布')
    expect(meta.description).toContain('下架')
    expect(meta.shouldRefresh).toBe(false)
  })

  it('删除 404 → 提示已不存在并要求刷新', () => {
    const meta = imageDeleteErrorMeta({ status: 404 })
    expect(meta.kind).toBe('not_found')
    expect(meta.shouldRefresh).toBe(true)
  })

  it('删除 403 → 权限提示', () => {
    const meta = imageDeleteErrorMeta({ status: 403 })
    expect(meta.kind).toBe('forbidden')
    expect(meta.description).toContain('image:manage')
  })

  it('其它错误保留后端消息，未知错误给兜底文案', () => {
    expect(imageDeleteErrorMeta(new Error('storage 503')).description).toBe('storage 503')
    expect(imageDeleteErrorMeta(new Error('')).description).toBe('请稍后重试')
  })

  it('下架错误：409 说明仅 published 可下架，404 要求刷新', () => {
    expect(imageDeprecateErrorMeta({ status: 409 }).description).toContain('published')
    expect(imageDeprecateErrorMeta({ status: 404 }).shouldRefresh).toBe(true)
    expect(imageDeprecateErrorMeta({ status: 500 }).title).toBe('下架镜像失败')
  })
})
