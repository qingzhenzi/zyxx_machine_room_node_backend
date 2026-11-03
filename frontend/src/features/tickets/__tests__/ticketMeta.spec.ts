import { describe, expect, it } from 'vitest'

import {
  attachmentSizeLabel,
  attachmentTypeLabel,
  availableActions,
  describeTicketEvent,
  isAllowedAttachmentType,
  isSha256Hex,
  normalizeRootCause,
  rootCauseLabel,
  TICKET_ATTACHMENT_MAX_BYTES,
  validateAttachmentFile,
  isActiveStatus,
  isSystemEvent,
  isTerminalStatus,
  ticketCategoryLabel,
  ticketCategoryMeta,
  ticketEventKindMeta,
  ticketMetadataHealth,
  ticketPriorityMeta,
  ticketSourceMeta,
  ticketStatusMeta,
} from '../ticketMeta'

describe('ticketMeta 文案与色调', () => {
  it('映射全部状态的中文文案与 StateBadge tone', () => {
    expect(ticketStatusMeta('open')).toEqual({ label: '待处理', tone: 'amber' })
    expect(ticketStatusMeta('in_progress')).toEqual({ label: '处理中', tone: 'accent' })
    expect(ticketStatusMeta('resolved')).toEqual({ label: '已解决', tone: 'emerald' })
    expect(ticketStatusMeta('closed')).toEqual({ label: '已关闭', tone: 'slate' })
    expect(ticketStatusMeta('cancelled')).toEqual({ label: '已取消', tone: 'slate' })
    expect(ticketStatusMeta(undefined)).toEqual({ label: '未知', tone: 'slate' })
    expect(ticketStatusMeta('weird')).toEqual({ label: 'weird', tone: 'slate' })
  })

  it('优先级带契约规定的 SLA 小时数', () => {
    expect(ticketPriorityMeta('urgent').label).toBe('紧急')
    expect(ticketPriorityMeta('urgent').slaHours).toBe(1)
    expect(ticketPriorityMeta('high').slaHours).toBe(4)
    expect(ticketPriorityMeta('normal').slaHours).toBe(24)
    expect(ticketPriorityMeta('low').slaHours).toBe(72)
    expect(ticketPriorityMeta(null).label).toBe('未知')
  })

  it('分类与来源文案', () => {
    expect(ticketCategoryLabel('machine_health')).toBe('机器健康')
    expect(ticketCategoryLabel('device_offline')).toBe('设备离线')
    expect(ticketCategoryLabel('other')).toBe('其他')
    expect(ticketCategoryMeta('security').barClass).toBe('bg-rose-600')
    expect(ticketSourceMeta('auto').label).toBe('自动')
    expect(ticketSourceMeta('manual').label).toBe('人工')
    expect(ticketEventKindMeta('occurred').label).toBe('再次触发')
  })

  it('判断活跃 / 终态状态', () => {
    expect(isActiveStatus('open')).toBe(true)
    expect(isActiveStatus('in_progress')).toBe(true)
    expect(isActiveStatus('resolved')).toBe(false)
    expect(isTerminalStatus('closed')).toBe(true)
    expect(isTerminalStatus('cancelled')).toBe(true)
    expect(isTerminalStatus('open')).toBe(false)
  })
})

describe('availableActions 状态机', () => {
  it('无 ticket:handle 权限时不展示任何动作', () => {
    expect(availableActions('open', false)).toEqual([])
    expect(availableActions('in_progress', false)).toEqual([])
  })

  it('待处理：开始处理 / 认领 / 取消', () => {
    const actions = availableActions('open', true)
    expect(actions.map((action) => action.key)).toEqual(['start', 'claim', 'cancel'])
    expect(actions[0]?.toStatus).toBe('in_progress')
    expect(actions[1]?.viaClaim).toBe(true)
    expect(actions[2]?.needsConfirm).toBe(true)
  })

  it('处理中：标记解决必须填解决说明 + 取消', () => {
    const actions = availableActions('in_progress', true)
    expect(actions.map((action) => action.key)).toEqual(['resolve', 'cancel'])
    expect(actions[0]?.toStatus).toBe('resolved')
    expect(actions[0]?.needsResolution).toBe(true)
  })

  it('已解决：关闭需确认 + 重开', () => {
    const actions = availableActions('resolved', true)
    expect(actions.map((action) => action.key)).toEqual(['close', 'reopen'])
    expect(actions[0]?.toStatus).toBe('closed')
    expect(actions[0]?.needsConfirm).toBe(true)
    expect(actions[1]?.toStatus).toBe('open')
  })

  it('已关闭 / 已取消只能重开', () => {
    for (const status of ['closed', 'cancelled'] as const) {
      const actions = availableActions(status, true)
      expect(actions).toHaveLength(1)
      expect(actions[0]?.key).toBe('reopen')
      expect(actions[0]?.toStatus).toBe('open')
    }
  })

  it('未知状态不返回动作', () => {
    expect(availableActions('weird', true)).toEqual([])
    expect(availableActions(null, true)).toEqual([])
  })
})

describe('describeTicketEvent 时间线文案', () => {
  it('状态变更显示 from → to', () => {
    expect(
      describeTicketEvent({
        kind: 'status_changed',
        from_status: 'open',
        to_status: 'in_progress',
      }),
    ).toBe('状态 待处理 → 处理中')
  })

  it('内容更新事件（PATCH 标题 / 描述产生）', () => {
    expect(describeTicketEvent({ kind: 'updated' })).toBe('更新了工单内容')
    expect(ticketEventKindMeta('updated').label).toBe('更新内容')
  })

  it('创建 / 认领 / 评论事件', () => {
    expect(describeTicketEvent({ kind: 'created' })).toBe('创建了工单')
    expect(describeTicketEvent({ kind: 'claimed' })).toBe('认领了工单，开始处理')
    expect(describeTicketEvent({ kind: 'commented', note: '已重启服务' })).toBe('发表了评论')
  })

  it('重复触发从 payload 读取 occurrence_count', () => {
    expect(describeTicketEvent({ kind: 'occurred', payload: { occurrence_count: 3 } })).toBe(
      '同一问题再次触发（累计 3 次）',
    )
    expect(describeTicketEvent({ kind: 'occurred', payload: { count: 2 } })).toBe(
      '同一问题再次触发（累计 2 次）',
    )
    expect(describeTicketEvent({ kind: 'occurred' })).toBe('同一问题再次触发')
  })

  it('未知 kind 回退成中性文案', () => {
    expect(describeTicketEvent({ kind: 'unknown' })).toContain('unknown')
    expect(describeTicketEvent({})).toContain('事件')
  })
})

describe('isSystemEvent', () => {
  it('系统事件与人工评论区分', () => {
    expect(isSystemEvent({ actor_name: '系统', kind: 'created' })).toBe(true)
    expect(isSystemEvent({ actor_name: '张伟', kind: 'commented' })).toBe(false)
    expect(isSystemEvent({ kind: 'commented' })).toBe(false)
    expect(isSystemEvent({ kind: 'occurred' })).toBe(true)
  })
})

describe('ticketMetadataHealth', () => {
  it('从自动建单 metadata 提取健康状态', () => {
    expect(ticketMetadataHealth({ health_status: 'critical' })).toBe('critical')
    expect(ticketMetadataHealth({ machine_health_status: 'warning' })).toBe('warning')
    expect(ticketMetadataHealth(null)).toBeNull()
    expect(ticketMetadataHealth('not-an-object')).toBeNull()
    expect(ticketMetadataHealth({})).toBeNull()
  })
})

describe('根因分类映射', () => {
  it('归一化合法根因并输出中文', () => {
    expect(normalizeRootCause('hardware')).toBe('hardware')
    expect(normalizeRootCause('network')).toBe('network')
    expect(normalizeRootCause('unknown')).toBeNull()
    expect(normalizeRootCause(null)).toBeNull()
    expect(rootCauseLabel('hardware')).toBe('硬件故障')
    expect(rootCauseLabel('config')).toBe('配置错误')
    expect(rootCauseLabel('weird')).toBe('weird')
    expect(rootCauseLabel(null)).toBe('—')
  })
})

describe('附件校验', () => {
  it('content_type 白名单', () => {
    expect(isAllowedAttachmentType('image/png')).toBe(true)
    expect(isAllowedAttachmentType('application/pdf')).toBe(true)
    expect(isAllowedAttachmentType('text/plain')).toBe(true)
    expect(isAllowedAttachmentType('application/zip')).toBe(true)
    expect(isAllowedAttachmentType('application/gzip')).toBe(true)
    expect(isAllowedAttachmentType('application/octet-stream')).toBe(true)
    expect(isAllowedAttachmentType('text/html')).toBe(false)
    expect(isAllowedAttachmentType('')).toBe(false)
  })

  it('文件名 / 大小 / 类型校验', () => {
    expect(
      validateAttachmentFile({ filename: 'log.txt', size: 1024, contentType: 'text/plain' }),
    ).toBe('')
    expect(
      validateAttachmentFile({ filename: 'shot.png', size: 2048, contentType: 'image/png' }),
    ).toBe('')
    expect(validateAttachmentFile({ filename: '', size: 1, contentType: 'text/plain' })).toContain(
      '请选择文件',
    )
    expect(
      validateAttachmentFile({ filename: 'a.txt', size: 0, contentType: 'text/plain' }),
    ).toContain('空文件')
    expect(
      validateAttachmentFile({
        filename: 'big.zip',
        size: TICKET_ATTACHMENT_MAX_BYTES + 1,
        contentType: 'application/zip',
      }),
    ).toContain('不能超过')
    expect(
      validateAttachmentFile({ filename: 'page.html', size: 10, contentType: 'text/html' }),
    ).toContain('仅支持')
    expect(
      validateAttachmentFile({
        filename: 'x'.repeat(256),
        size: 10,
        contentType: 'text/plain',
      }),
    ).toContain('文件名不能超过')
  })

  it('sha256 与展示辅助', () => {
    expect(isSha256Hex('a'.repeat(64))).toBe(true)
    expect(isSha256Hex('A'.repeat(64))).toBe(false)
    expect(isSha256Hex('abc')).toBe(false)
    expect(attachmentSizeLabel(2048)).toBe('2.00 KiB')
    expect(attachmentSizeLabel(null)).toBe('—')
    expect(attachmentTypeLabel('image/png')).toBe('图片')
    expect(attachmentTypeLabel('application/pdf')).toBe('PDF')
    expect(attachmentTypeLabel('text/plain')).toBe('文本')
    expect(attachmentTypeLabel('application/zip')).toBe('压缩包')
  })
})
