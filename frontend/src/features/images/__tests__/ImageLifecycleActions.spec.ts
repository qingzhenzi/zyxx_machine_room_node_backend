import { mount } from '@vue/test-utils'
import { createPinia } from 'pinia'
import { nextTick } from 'vue'
import { describe, expect, it } from 'vitest'

import type { ImageArtifact } from '@/api/schema'

import ImageLifecycleActions from '../ImageLifecycleActions.vue'

function makeImage(status: ImageArtifact['status']): ImageArtifact {
  return {
    id: '11111111-1111-1111-1111-111111111111',
    name: 'ubuntu',
    version: '22.04',
    kind: 'vm_disk',
    format: 'qcow2',
    compression: 'none',
    platform: 'linux/amd64',
    status,
    total_size: 1024,
    manifest_digest: null,
    created_by: null,
    created_at: '2026-11-07T10:00:00Z',
    updated_at: '2026-11-07T10:00:00Z',
  }
}

function mountActions(status: ImageArtifact['status']) {
  return mount(ImageLifecycleActions, {
    props: { image: makeImage(status), size: 'sm' },
    global: { plugins: [createPinia()] },
    attachTo: document.body,
  })
}

describe('ImageLifecycleActions', () => {
  it('published：显示「下架」与「删除」按钮', () => {
    const wrapper = mountActions('published')
    expect(wrapper.text()).toContain('下架')
    expect(wrapper.text()).toContain('删除')
    wrapper.unmount()
  })

  it('deprecated：不再显示「下架」，只保留「删除」', () => {
    const wrapper = mountActions('deprecated')
    expect(wrapper.text()).not.toContain('下架')
    expect(wrapper.text()).toContain('删除')
    wrapper.unmount()
  })

  it('点击「删除」弹出危险确认弹窗（含 S3 清理 / 不可恢复文案）', async () => {
    const wrapper = mountActions('verified')
    const deleteButton = wrapper.findAll('button').find((button) => button.text() === '删除')
    expect(deleteButton).toBeDefined()
    await deleteButton!.trigger('click')
    await nextTick()

    const dialogText = document.body.textContent ?? ''
    expect(dialogText).toContain('删除镜像')
    expect(dialogText).toContain('S3 对象')
    expect(dialogText).toContain('不可恢复')
    wrapper.unmount()
  })
})
