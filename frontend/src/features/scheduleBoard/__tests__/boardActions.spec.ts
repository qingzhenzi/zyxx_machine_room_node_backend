import { describe, expect, it } from 'vitest'

import type {
  BoardBlock,
  BoardException,
  BoardPayload,
  BoardRoom,
  BoardSchedule,
  BoardWeeklySlot,
} from '../boardTypes'
import {
  buildMatrixClipboard,
  effectiveDaySlotsOf,
  exceptionPlanRefs,
  formToBoardDaySlot,
  planBatchClear,
  planBatchFill,
  planBatchPaste,
  planDayException,
  planOccurrenceCancel,
  planOccurrenceCreate,
  planOccurrenceMove,
  planOccurrenceRestore,
  snapshotExceptions,
  snapshotTemplates,
  templateDaySlotsOf,
  validateDaySlots,
  type BoardDaySlot,
  type BoardSlotFormValue,
  type ExceptionPlan,
  type MatrixCellRef,
} from '../boardActions'

const AREA_ID = 'area-1'
const ROOM_A = 'room-a'
const ROOM_B = 'room-b'
const SCHEDULE_A = 'schedule-a'
const SCHEDULE_B = 'schedule-b'
/** 2026-11-02 是周一。 */
const MONDAY = '2026-11-02'

function scheduleOf(id: string, roomId: string): BoardSchedule {
  return {
    id,
    room_id: roomId,
    name: '默认课表',
    start_date: '2026-09-01',
    end_date: '2027-01-31',
    is_active: true,
    timezone: 'Asia/Shanghai',
  }
}

function weeklySlot(
  id: string,
  scheduleId: string,
  startTime: string,
  endTime: string,
  courseName: string,
  teacher: string,
  dayOfWeek = 1,
  roomId: string | null = null,
): BoardWeeklySlot {
  return {
    id,
    schedule_id: scheduleId,
    day_of_week: dayOfWeek,
    start_time: startTime,
    end_time: endTime,
    course_name: courseName,
    teacher,
    room_id: roomId,
    note: '',
  }
}

function exceptionOf(
  id: string,
  scheduleId: string,
  date: string,
  kind: 'cancel' | 'replace',
  slots: BoardException['slots'],
): BoardException {
  return { id, schedule_id: scheduleId, exception_date: date, kind, note: '', slots }
}

function daySlot(
  startTime: string,
  endTime: string,
  courseName: string,
  teacher = '',
  roomId: string | null = null,
  slotId?: string,
): BoardDaySlot {
  return {
    start_time: startTime,
    end_time: endTime,
    course_name: courseName,
    teacher,
    room_id: roomId,
    note: '',
    slot_id: slotId,
  }
}

function blockOf(params: {
  scheduleId: string
  roomId: string
  startTime: string
  endTime: string
  courseName: string
  teacher?: string
  source?: 'template' | 'exception'
  slotId: string
}): BoardBlock {
  return {
    key: `${params.source === 'exception' ? 'exception' : 'template'}:${params.slotId}:${MONDAY}`,
    source: params.source ?? 'template',
    schedule_id: params.scheduleId,
    slot_id: params.slotId,
    room_id: params.roomId,
    room_code: params.roomId,
    room_name: params.roomId,
    area_id: AREA_ID,
    area_name: '演示区',
    date: MONDAY,
    day_of_week: 1,
    start_time: params.startTime,
    end_time: params.endTime,
    course_name: params.courseName,
    teacher: params.teacher ?? '',
    note: '',
    is_override: params.roomId !== params.scheduleId,
    conflicts: [],
  }
}

const WEEKLY_SLOTS: BoardWeeklySlot[] = [
  weeklySlot('week-a-1', SCHEDULE_A, '08:00', '09:40', '高等数学', '张伟'),
  weeklySlot('week-a-2', SCHEDULE_A, '10:00', '11:40', '英语', '李娜'),
  weeklySlot('week-b-1', SCHEDULE_B, '15:30', '17:10', '物理', '王强'),
]

function makePayload(overrides: Partial<BoardPayload> = {}): BoardPayload {
  return {
    from: MONDAY,
    to: '2026-11-08',
    areas: [],
    rooms: [],
    schedules: [scheduleOf(SCHEDULE_A, ROOM_A), scheduleOf(SCHEDULE_B, ROOM_B)],
    weekly_slots: WEEKLY_SLOTS.map((slot) => ({ ...slot })),
    exceptions: [],
    ...overrides,
  }
}

function formValue(overrides: Partial<BoardSlotFormValue> = {}): BoardSlotFormValue {
  return {
    course_name: '体育',
    teacher: '王教练',
    room_id: ROOM_A,
    day_of_week: 1,
    start_time: '13:30',
    end_time: '15:10',
    note: '',
    ...overrides,
  }
}

describe('templateDaySlotsOf / effectiveDaySlotsOf', () => {
  it('按 start_time 排序返回周模板当天时段，并带模板 slot_id', () => {
    const slots = templateDaySlotsOf(makePayload(), SCHEDULE_A, 1)
    expect(slots.map((slot) => slot.course_name)).toEqual(['高等数学', '英语'])
    expect(slots[0]?.slot_id).toBe('week-a-1')
    expect(templateDaySlotsOf(makePayload(), SCHEDULE_A, 2)).toEqual([])
  })

  it('有 replace 覆盖时以覆盖为准（带覆盖 slot_id），cancel 覆盖返回空', () => {
    const replace = makePayload({
      exceptions: [
        exceptionOf('exc-1', SCHEDULE_A, MONDAY, 'replace', [
          {
            id: 'exc-slot-1',
            start_time: '15:30',
            end_time: '17:10',
            course_name: '化学',
            teacher: '赵六',
            room_id: null,
            note: '',
          },
        ]),
      ],
    })
    const effective = effectiveDaySlotsOf(replace, SCHEDULE_A, MONDAY, 1)
    expect(effective).toHaveLength(1)
    expect(effective[0]?.course_name).toBe('化学')
    expect(effective[0]?.slot_id).toBe('exc-slot-1')

    const cancel = makePayload({
      exceptions: [exceptionOf('exc-2', SCHEDULE_A, MONDAY, 'cancel', [])],
    })
    expect(effectiveDaySlotsOf(cancel, SCHEDULE_A, MONDAY, 1)).toEqual([])
  })
})

describe('formToBoardDaySlot', () => {
  it('房间等于课表房间时 room_id 为 null，文本字段 trim', () => {
    const slot = formToBoardDaySlot(
      formValue({ course_name: ' 体育 ', teacher: ' 王教练 ', note: ' 备注 ' }),
      ROOM_A,
    )
    expect(slot).toMatchObject({
      course_name: '体育',
      teacher: '王教练',
      note: '备注',
      room_id: null,
    })
  })

  it('房间不是课表房间时保留调课 room_id', () => {
    const slot = formToBoardDaySlot(formValue({ room_id: ROOM_B }), ROOM_A)
    expect(slot.room_id).toBe(ROOM_B)
  })
})

describe('validateDaySlots', () => {
  it('时间顺序或重叠时返回错误文案', () => {
    expect(validateDaySlots([daySlot('11:00', '10:00', 'A')])).toContain('开始时间必须早于结束时间')
    expect(
      validateDaySlots([daySlot('10:00', '11:00', 'A'), daySlot('09:30', '10:30', 'B')]),
    ).toContain('重叠')
    expect(validateDaySlots([daySlot('08:00', '09:40', 'A'), daySlot('10:00', '11:40', 'B')])).toBe(
      '',
    )
  })

  it('超过 100 条时拒绝', () => {
    const tooMany = Array.from({ length: 101 }, (_, index) =>
      daySlot(
        `${String(8 + Math.floor(index / 60)).padStart(2, '0')}:${String(index % 60).padStart(2, '0')}`,
        '23:59',
        `课程${index}`,
      ),
    )
    expect(validateDaySlots(tooMany)).toContain('最多')
  })
})

describe('planDayException', () => {
  const payload = makePayload()
  const template = templateDaySlotsOf(payload, SCHEDULE_A, 1)

  it('模板日删除一节课 → replace 剩余时段', () => {
    const plan = planDayException(payload, SCHEDULE_A, MONDAY, 1, template.slice(1))
    expect(plan).toMatchObject({ schedule_id: SCHEDULE_A, date: MONDAY, action: 'replace' })
    expect(plan.slots).toEqual([
      {
        start_time: '10:00',
        end_time: '11:40',
        course_name: '英语',
        teacher: '李娜',
        room_id: null,
        note: '',
      },
    ])
  })

  it('删空 → cancel', () => {
    const plan = planDayException(payload, SCHEDULE_A, MONDAY, 1, [])
    expect(plan.action).toBe('cancel')
    expect(plan.slots).toEqual([])
  })

  it('改回模板等值（含 trim 差异）→ delete 恢复模板', () => {
    const next = template.map((slot) => ({
      ...slot,
      course_name: ` ${slot.course_name} `,
      teacher: `${slot.teacher} `,
    }))
    const plan = planDayException(payload, SCHEDULE_A, MONDAY, 1, next)
    expect(plan.action).toBe('delete')
    expect(plan.slots).toEqual([])
  })

  it('已有 replace 覆盖时继承当天现状再改', () => {
    const withReplace = makePayload({
      exceptions: [
        exceptionOf('exc-1', SCHEDULE_A, MONDAY, 'replace', [
          {
            id: 'exc-slot-1',
            start_time: '15:30',
            end_time: '17:10',
            course_name: '化学',
            teacher: '赵六',
            room_id: null,
            note: '',
          },
        ]),
      ],
    })
    const current = effectiveDaySlotsOf(withReplace, SCHEDULE_A, MONDAY, 1)
    const plan = planDayException(withReplace, SCHEDULE_A, MONDAY, 1, [
      ...current,
      daySlot('18:30', '20:10', '晚辅', '', null),
    ])
    expect(plan.action).toBe('replace')
    expect(plan.slots.map((slot) => slot.course_name)).toEqual(['化学', '晚辅'])
    // 覆盖里唯一的课时被取消 → 整天 cancel（模板当天非空）
    expect(planDayException(withReplace, SCHEDULE_A, MONDAY, 1, []).action).toBe('cancel')
  })
})

describe('planOccurrenceCancel / planOccurrenceRestore', () => {
  it('停课当天一节课：剩余时段写 replace；当天只有一节时落为 cancel', () => {
    const payload = makePayload()
    const plan = planOccurrenceCancel(
      payload,
      blockOf({
        scheduleId: SCHEDULE_A,
        roomId: ROOM_A,
        startTime: '08:00',
        endTime: '09:40',
        courseName: '高等数学',
        teacher: '张伟',
        slotId: 'week-a-1',
      }),
    )
    expect(plan).toHaveLength(1)
    expect(plan[0]?.action).toBe('replace')
    expect(plan[0]?.slots.map((slot) => slot.course_name)).toEqual(['英语'])

    const single = makePayload({
      exceptions: [
        exceptionOf('exc-1', SCHEDULE_B, MONDAY, 'replace', [
          {
            id: 'exc-slot-b',
            start_time: '13:30',
            end_time: '15:10',
            course_name: '化学',
            teacher: '',
            room_id: null,
            note: '',
          },
        ]),
      ],
    })
    const only = planOccurrenceCancel(
      single,
      blockOf({
        scheduleId: SCHEDULE_B,
        roomId: ROOM_B,
        startTime: '13:30',
        endTime: '15:10',
        courseName: '化学',
        source: 'exception',
        slotId: 'exc-slot-b',
      }),
    )
    expect(only[0]?.action).toBe('cancel')
  })

  it('恢复本次删除当天覆盖；模板块没有覆盖时不产生计划', () => {
    const payload = makePayload({
      exceptions: [
        exceptionOf('exc-1', SCHEDULE_A, MONDAY, 'replace', [
          {
            id: 'exc-slot-1',
            start_time: '15:30',
            end_time: '17:10',
            course_name: '化学',
            teacher: '',
            room_id: null,
            note: '',
          },
        ]),
      ],
    })
    const restore = planOccurrenceRestore(
      payload,
      blockOf({
        scheduleId: SCHEDULE_A,
        roomId: ROOM_A,
        startTime: '15:30',
        endTime: '17:10',
        courseName: '化学',
        source: 'exception',
        slotId: 'exc-slot-1',
      }),
    )
    expect(restore).toHaveLength(1)
    expect(restore[0]?.action).toBe('delete')

    const templateBlock = blockOf({
      scheduleId: SCHEDULE_A,
      roomId: ROOM_A,
      startTime: '08:00',
      endTime: '09:40',
      courseName: '高等数学',
      teacher: '张伟',
      slotId: 'week-a-1',
    })
    expect(planOccurrenceRestore(makePayload(), templateBlock)).toEqual([])
  })
})

describe('planOccurrenceMove 跨房间单日移动', () => {
  it('源课表移除该节、目标课表加入该节，各一条计划；目标课表 room 语义为 null', () => {
    const payload = makePayload()
    const sourceBlock = blockOf({
      scheduleId: SCHEDULE_A,
      roomId: ROOM_A,
      startTime: '08:00',
      endTime: '09:40',
      courseName: '高等数学',
      teacher: '张伟',
      slotId: 'week-a-1',
    })
    const plans = planOccurrenceMove(
      payload,
      sourceBlock,
      { room_id: ROOM_B, start_time: '13:30', end_time: '15:10' },
      scheduleOf(SCHEDULE_B, ROOM_B),
    )
    expect(plans).toHaveLength(2)

    const [sourcePlan, targetPlan] = plans
    expect(sourcePlan?.schedule_id).toBe(SCHEDULE_A)
    expect(sourcePlan?.action).toBe('replace')
    expect(sourcePlan?.slots.map((slot) => slot.course_name)).toEqual(['英语'])

    expect(targetPlan?.schedule_id).toBe(SCHEDULE_B)
    expect(targetPlan?.action).toBe('replace')
    const moved = targetPlan?.slots.find((slot) => slot.course_name === '高等数学')
    expect(moved).toBeDefined()
    expect(moved?.room_id).toBeNull()
    expect(moved?.teacher).toBe('张伟')
    // 目标房间已有课表当天时段保留
    expect(targetPlan?.slots.map((slot) => slot.course_name)).toEqual(['高等数学', '物理'])
  })

  it('同课表内改时间只产生一条计划；源课表当天只剩空时落为 cancel', () => {
    const payload = makePayload({
      weekly_slots: [weeklySlot('week-a-1', SCHEDULE_A, '08:00', '09:40', '高等数学', '张伟')],
    })
    const sourceBlock = blockOf({
      scheduleId: SCHEDULE_A,
      roomId: ROOM_A,
      startTime: '08:00',
      endTime: '09:40',
      courseName: '高等数学',
      teacher: '张伟',
      slotId: 'week-a-1',
    })
    const sameSchedulePlans = planOccurrenceMove(
      payload,
      sourceBlock,
      { room_id: ROOM_A, start_time: '10:00', end_time: '11:40' },
      scheduleOf(SCHEDULE_A, ROOM_A),
    )
    expect(sameSchedulePlans).toHaveLength(1)
    expect(sameSchedulePlans[0]?.action).toBe('replace')

    const crossPlans = planOccurrenceMove(
      payload,
      sourceBlock,
      { room_id: ROOM_B, start_time: '13:30', end_time: '15:10' },
      scheduleOf(SCHEDULE_B, ROOM_B),
    )
    expect(crossPlans[0]?.action).toBe('cancel')
    expect(crossPlans[1]?.action).toBe('replace')
  })
})

describe('planOccurrenceCreate', () => {
  it('当天有效时段 + 新时段 → replace；与模板完全一致时恢复为 delete', () => {
    const payload = makePayload()
    const schedule = scheduleOf(SCHEDULE_A, ROOM_A)
    const plans = planOccurrenceCreate(payload, schedule, MONDAY, 1, formValue())
    expect(plans).toHaveLength(1)
    expect(plans[0]?.action).toBe('replace')
    expect(plans[0]?.slots.map((slot) => slot.course_name)).toEqual(['高等数学', '英语', '体育'])
    expect(plans[0]?.slots[2]?.room_id).toBeNull()
  })
})

describe('planOccurrenceRestore 跨房间单日调课的恢复', () => {
  it('恢复目标课块时同时把源课表当天的这节课加回原位', () => {
    const payload = makePayload({
      exceptions: [
        exceptionOf('ex-a', SCHEDULE_A, MONDAY, 'replace', [
          {
            id: 'ex-a-1',
            start_time: '10:00',
            end_time: '11:40',
            course_name: '英语',
            teacher: '李娜',
            room_id: null,
            note: '',
          },
        ]),
        exceptionOf('ex-b', SCHEDULE_B, MONDAY, 'replace', [
          {
            id: 'ex-b-1',
            start_time: '13:30',
            end_time: '15:10',
            course_name: '高等数学',
            teacher: '张伟',
            room_id: null,
            note: '',
          },
          {
            id: 'ex-b-2',
            start_time: '15:30',
            end_time: '17:10',
            course_name: '物理',
            teacher: '王强',
            room_id: null,
            note: '',
          },
        ]),
      ],
    })
    const movedBlock = blockOf({
      scheduleId: SCHEDULE_B,
      roomId: ROOM_B,
      startTime: '13:30',
      endTime: '15:10',
      courseName: '高等数学',
      teacher: '张伟',
      source: 'exception',
      slotId: 'ex-b-1',
    })

    const plans = planOccurrenceRestore(payload, movedBlock)
    // 目标课表恢复为模板 + 源课表把调出的课加回
    expect(plans).toHaveLength(2)
    expect(plans.map((plan) => plan.schedule_id).sort()).toEqual([SCHEDULE_A, SCHEDULE_B].sort())
    // 两边都恢复成与模板一致 → delete 覆盖
    expect(plans.every((plan) => plan.action === 'delete')).toBe(true)
  })

  it('同房间内改时间的本次恢复只产生目标课表一条计划', () => {
    const payload = makePayload({
      exceptions: [
        exceptionOf('ex-a', SCHEDULE_A, MONDAY, 'replace', [
          {
            id: 'ex-a-1',
            start_time: '08:00',
            end_time: '09:40',
            course_name: '高等数学',
            teacher: '张伟',
            room_id: null,
            note: '',
          },
          {
            id: 'ex-a-2',
            start_time: '10:00',
            end_time: '11:40',
            course_name: '英语',
            teacher: '李娜',
            room_id: null,
            note: '',
          },
        ]),
      ],
    })
    const block = blockOf({
      scheduleId: SCHEDULE_A,
      roomId: ROOM_A,
      startTime: '10:00',
      endTime: '11:40',
      courseName: '英语',
      teacher: '李娜',
      source: 'exception',
      slotId: 'ex-a-2',
    })
    const plans = planOccurrenceRestore(payload, block)
    expect(plans).toHaveLength(1)
    expect(plans[0]?.schedule_id).toBe(SCHEDULE_A)
    // 恢复 = 这一天的覆盖删除、回到周模板
    expect(plans[0]?.action).toBe('delete')
  })
})

describe('撤销/重做快照', () => {
  it('snapshotTemplates 取各课表当前周模板并按课表去重', () => {
    const payload = makePayload()
    const snapshots = snapshotTemplates(payload, [SCHEDULE_A, SCHEDULE_B, SCHEDULE_A])
    expect(snapshots).toHaveLength(2)
    expect(snapshots[0]?.schedule_id).toBe(SCHEDULE_A)
    expect(snapshots[0]?.slots.map((slot) => slot.course_name)).toEqual(['高等数学', '英语'])
    expect(snapshots[1]?.schedule_id).toBe(SCHEDULE_B)
    expect(snapshots[1]?.slots.map((slot) => slot.course_name)).toEqual(['物理'])
    // 快照是 NewWeeklySlot（不含数据库 id），重放时可直接全量 PUT。
    expect(snapshots[0]?.slots[0]).not.toHaveProperty('id')
    expect(snapshots[0]?.slots[0]).toMatchObject({
      day_of_week: 1,
      start_time: '08:00',
      end_time: '09:40',
      room_id: null,
    })
    expect(snapshotTemplates(null, [SCHEDULE_A])[0]?.slots).toEqual([])
  })

  it('snapshotExceptions 区分无覆盖 null / replace / cancel，并带 note 与时段', () => {
    const replaceException = exceptionOf('exc-1', SCHEDULE_A, MONDAY, 'replace', [
      {
        id: 'exc-slot-1',
        start_time: '15:30',
        end_time: '17:10',
        course_name: '化学',
        teacher: '赵六',
        room_id: null,
        note: '调课',
      },
    ])
    replaceException.note = '临时调课'
    const payload = makePayload({ exceptions: [replaceException] })
    const snapshots = snapshotExceptions(payload, [
      { schedule_id: SCHEDULE_A, date: MONDAY },
      { schedule_id: SCHEDULE_B, date: MONDAY },
      { schedule_id: SCHEDULE_A, date: MONDAY },
    ])
    // refs 去重后只返回两条（A 的重复 ref 合并）
    expect(snapshots).toHaveLength(2)
    expect(snapshots[0]).toMatchObject({
      schedule_id: SCHEDULE_A,
      date: MONDAY,
      kind: 'replace',
      note: '临时调课',
    })
    expect(snapshots[0]?.slots).toEqual([
      {
        start_time: '15:30',
        end_time: '17:10',
        course_name: '化学',
        teacher: '赵六',
        room_id: null,
        note: '调课',
      },
    ])
    expect(snapshots[1]).toMatchObject({ schedule_id: SCHEDULE_B, kind: null, slots: [] })

    const cancelPayload = makePayload({
      exceptions: [exceptionOf('exc-2', SCHEDULE_A, MONDAY, 'cancel', [])],
    })
    expect(
      snapshotExceptions(cancelPayload, [{ schedule_id: SCHEDULE_A, date: MONDAY }])[0]?.kind,
    ).toBe('cancel')
  })

  it('exceptionPlanRefs 按 (schedule_id, date) 去重并保持首次出现顺序', () => {
    const plans: ExceptionPlan[] = [
      { schedule_id: SCHEDULE_A, date: MONDAY, action: 'replace', slots: [] },
      { schedule_id: SCHEDULE_A, date: MONDAY, action: 'cancel', slots: [] },
      { schedule_id: SCHEDULE_B, date: MONDAY, action: 'replace', slots: [] },
      { schedule_id: SCHEDULE_A, date: '2026-11-03', action: 'replace', slots: [] },
    ]
    expect(exceptionPlanRefs(plans)).toEqual([
      { schedule_id: SCHEDULE_A, date: MONDAY },
      { schedule_id: SCHEDULE_B, date: MONDAY },
      { schedule_id: SCHEDULE_A, date: '2026-11-03' },
    ])
  })
})

describe('框选批量：planBatchFill / planBatchClear', () => {
  const fillValue = { course_name: ' 体育 ', teacher: ' 吴磊 ', note: ' 批量 ' }

  function cell(roomId: string, day: number, period: number): MatrixCellRef {
    return { room_id: roomId, day_of_week: day, period_index: period }
  }

  it('仅填充空格：跳过占用格，只给空格子新增固定节次时段', () => {
    const result = planBatchFill(
      makePayload(),
      [cell(ROOM_A, 1, 0), cell(ROOM_A, 1, 2)],
      fillValue,
      'empty',
    )
    expect(result.created).toBe(1)
    expect(result.skipped).toBe(1)
    expect(result.errors).toEqual([])
    expect(result.perRoom).toEqual([
      { room_id: ROOM_A, schedule_id: SCHEDULE_A, created: 1, skipped: 1 },
    ])
    expect(result.plans).toHaveLength(1)
    const slots = result.plans[0]?.slots ?? []
    expect(slots.map((slot) => slot.course_name)).toEqual(['高等数学', '英语', '体育'])
    expect(slots[2]).toMatchObject({
      day_of_week: 1,
      start_time: '13:30',
      end_time: '15:10',
      teacher: '吴磊',
      room_id: null,
      note: '批量',
    })
  })

  it('覆盖已有课：先移除占用 slot 再追加，格子数不变', () => {
    const result = planBatchFill(makePayload(), [cell(ROOM_A, 1, 0)], fillValue, 'overwrite')
    expect(result.created).toBe(1)
    expect(result.skipped).toBe(0)
    const slots = result.plans[0]?.slots ?? []
    expect(slots.some((slot) => slot.course_name === '高等数学')).toBe(false)
    expect(slots.map((slot) => slot.course_name)).toEqual(['英语', '体育'])
  })

  it('批量清除移除同房间同节次的模板课程，其它课程不动', () => {
    const result = planBatchClear(makePayload(), [cell(ROOM_A, 1, 0)])
    expect(result.cleared).toBe(1)
    expect(result.plans).toHaveLength(1)
    expect(result.plans[0]?.slots.map((slot) => slot.course_name)).toEqual(['英语'])

    const noop = planBatchClear(makePayload(), [cell(ROOM_A, 3, 0)])
    expect(noop.cleared).toBe(0)
    expect(noop.plans).toEqual([])
  })

  it('跨房间选中 → 每个课表一条计划；重复格子去重', () => {
    const result = planBatchFill(
      makePayload(),
      [cell(ROOM_A, 2, 0), cell(ROOM_A, 2, 0), cell(ROOM_B, 1, 0)],
      fillValue,
      'empty',
    )
    expect(result.created).toBe(2)
    expect(result.perRoom.map((room) => room.schedule_id)).toEqual([SCHEDULE_A, SCHEDULE_B])
    expect(result.plans).toHaveLength(2)
    for (const plan of result.plans) {
      expect(plan.slots.filter((slot) => slot.course_name === '体育')).toHaveLength(1)
    }
  })

  it('新增时段与已有课程重叠时报 errors', () => {
    const payload = makePayload({
      weekly_slots: [weeklySlot('week-a-1', SCHEDULE_A, '08:00', '10:30', '高等数学', '张伟')],
    })
    const result = planBatchFill(payload, [cell(ROOM_A, 1, 1)], fillValue, 'empty')
    expect(result.created).toBe(1)
    expect(result.errors).toHaveLength(1)
    expect(result.errors[0]).toContain('重叠')
  })

  it('房间还没有课表时计入新建数但暂不产出计划（等待 ensure 后重算）', () => {
    const result = planBatchFill(
      makePayload(),
      [cell('room-without-schedule', 1, 0)],
      fillValue,
      'empty',
    )
    expect(result.created).toBe(1)
    expect(result.plans).toEqual([])
    expect(result.perRoom).toEqual([
      { room_id: 'room-without-schedule', schedule_id: '', created: 1, skipped: 0 },
    ])
  })
})

describe('C 矩阵复制/粘贴：buildMatrixClipboard / planBatchPaste', () => {
  function roomOf(id: string, code: string): BoardRoom {
    return {
      id,
      area_id: AREA_ID,
      area_name: '演示区',
      code,
      name: code,
      capacity: 30,
      device_slot: null,
      is_active: true,
      timezone: 'Asia/Shanghai',
      device: null,
    }
  }
  const rooms = [roomOf(ROOM_A, 'a-room'), roomOf(ROOM_B, 'b-room')]
  function cell(roomId: string, day: number, period: number): MatrixCellRef {
    return { room_id: roomId, day_of_week: day, period_index: period }
  }

  it('整行复制后粘贴到另一房间：写入有课的格子', () => {
    const payload = makePayload()
    const cells = Array.from({ length: 5 }, (_, period) => cell(ROOM_A, 1, period))
    const clipboard = buildMatrixClipboard(payload, rooms, cells)
    expect(clipboard.rows).toEqual([ROOM_A])
    expect(clipboard.columns).toHaveLength(5)
    expect(clipboard.cells.map((item) => item.slots.map((slot) => slot.course_name))).toEqual([
      ['高等数学'],
      ['英语'],
    ])
    const result = planBatchPaste(payload, clipboard, cell(ROOM_B, 1, 0), rooms)
    expect(result.errors).toEqual([])
    expect(result.created).toBe(2)
    expect(result.overwritten).toBe(0)
    expect(result.skipped).toBe(0)
    expect(result.perRoom).toEqual([
      { room_id: ROOM_B, schedule_id: SCHEDULE_B, created: 2, overwritten: 0 },
    ])
    const slots = result.plans[0]?.slots ?? []
    const highMath = slots.find((slot) => slot.course_name === '高等数学')
    const english = slots.find((slot) => slot.course_name === '英语')
    expect(highMath).toMatchObject({
      day_of_week: 1,
      start_time: '08:00',
      end_time: '09:40',
      room_id: null,
    })
    expect(english).toMatchObject({ day_of_week: 1, start_time: '10:00', end_time: '11:40' })
    // 目标房间原有课程保留
    expect(slots.some((slot) => slot.course_name === '物理')).toBe(true)
  })

  it('矩形相对偏移 + 越界跳过', () => {
    const payload = makePayload({
      weekly_slots: [
        weeklySlot('week-a-1', SCHEDULE_A, '08:00', '09:40', '高等数学', '张伟'),
        weeklySlot('week-a-2', SCHEDULE_A, '10:00', '11:40', '英语', '李娜'),
        weeklySlot('week-b-1', SCHEDULE_B, '08:00', '09:40', '数据结构', '陈静'),
      ],
    })
    // 2 行 × 2 列：A 周一 p0/p1 + B 周一 p0；B 行贴到锚点将越界
    const cells = [cell(ROOM_A, 1, 0), cell(ROOM_A, 1, 1), cell(ROOM_B, 1, 0)]
    const clipboard = buildMatrixClipboard(payload, rooms, cells)
    expect(clipboard.rows).toEqual([ROOM_A, ROOM_B])
    expect(clipboard.columns).toHaveLength(2)
    const result = planBatchPaste(payload, clipboard, cell(ROOM_B, 1, 0), rooms)
    expect(result.skipped).toBe(1)
    expect(result.created).toBe(2)
    expect(result.overwritten).toBe(1) // 目标 B 周一 p0 原有「数据结构」被覆盖
    expect(result.errors).toEqual([])
  })

  it('目标格已有课被覆盖，源空格不清空目标格', () => {
    const payload = makePayload({
      weekly_slots: [
        weeklySlot('week-a-1', SCHEDULE_A, '08:00', '09:40', '高等数学', '张伟'),
        weeklySlot('week-b-1', SCHEDULE_B, '08:00', '09:40', '数据结构', '陈静'),
        weeklySlot('week-b-2', SCHEDULE_B, '10:00', '11:40', '计算机网络', '刘洋'),
      ],
    })
    // 剪贴板 1 行 × 2 列：只有 A 周一 p0 有课，p1 是空格
    const clipboard = buildMatrixClipboard(payload, rooms, [cell(ROOM_A, 1, 0), cell(ROOM_A, 1, 1)])
    expect(clipboard.columns).toHaveLength(2)
    expect(clipboard.cells).toHaveLength(1)
    const result = planBatchPaste(payload, clipboard, cell(ROOM_B, 1, 0), rooms)
    expect(result.created).toBe(1)
    expect(result.overwritten).toBe(1)
    const slots = result.plans[0]?.slots ?? []
    expect(
      slots.some((slot) => slot.course_name === '高等数学' && slot.start_time === '08:00'),
    ).toBe(true)
    // 复制源 p1 为空 → 目标 B 周一 p1 的「计算机网络」保持不动
    expect(slots.some((slot) => slot.course_name === '计算机网络')).toBe(true)
  })

  it('时间按「目标节次起点 + 源起点偏移」平移，时长不变', () => {
    const payload = makePayload({
      weekly_slots: [weeklySlot('week-a-1', SCHEDULE_A, '08:30', '10:10', '高等数学', '张伟')],
    })
    const clipboard = buildMatrixClipboard(payload, rooms, [cell(ROOM_A, 1, 0)])
    const result = planBatchPaste(payload, clipboard, cell(ROOM_B, 1, 1), rooms)
    const pasted = result.plans[0]?.slots.find((slot) => slot.course_name === '高等数学')
    expect(pasted).toMatchObject({
      day_of_week: 1,
      start_time: '10:30',
      end_time: '12:10',
    })
  })

  it('粘贴后同一天时段重叠时报 errors', () => {
    const payload = makePayload({
      weekly_slots: [
        weeklySlot('week-a-1', SCHEDULE_A, '08:00', '09:40', '高等数学', '张伟'),
        weeklySlot('week-b-1', SCHEDULE_B, '09:30', '11:00', '数据结构', '陈静'),
      ],
    })
    const clipboard = buildMatrixClipboard(payload, rooms, [cell(ROOM_A, 1, 0)])
    const result = planBatchPaste(payload, clipboard, cell(ROOM_B, 1, 1), rooms)
    expect(result.errors).toHaveLength(1)
    expect(result.errors[0]).toContain('重叠')
  })

  it('当天有单日覆盖的格子不算模板课块，不进入剪贴板', () => {
    const payload = makePayload({
      exceptions: [
        exceptionOf('exc-1', SCHEDULE_A, MONDAY, 'replace', [
          {
            id: 'exc-slot-1',
            start_time: '15:30',
            end_time: '17:10',
            course_name: '化学',
            teacher: '',
            room_id: null,
            note: '',
          },
        ]),
      ],
    })
    const clipboard = buildMatrixClipboard(payload, rooms, [cell(ROOM_A, 1, 0)])
    expect(clipboard.rows).toEqual([ROOM_A])
    expect(clipboard.cells).toEqual([])
  })

  it('目标房间还没有课表时计入 created、暂不产出计划', () => {
    const payload = makePayload()
    const clipboard = buildMatrixClipboard(payload, rooms, [cell(ROOM_A, 1, 0)])
    const result = planBatchPaste(payload, clipboard, cell('room-without-schedule', 1, 0), rooms)
    // 锚点不在 rooms 中 → 直接报错
    expect(result.errors.length).toBe(1)

    const withPendingRoom = [
      roomOf(ROOM_A, 'a-room'),
      roomOf(ROOM_B, 'b-room'),
      roomOf('room-c', 'c-room'),
    ]
    const pending = planBatchPaste(payload, clipboard, cell('room-c', 1, 0), withPendingRoom)
    expect(pending.created).toBe(1)
    expect(pending.plans).toEqual([])
    expect(pending.perRoom[0]).toMatchObject({ room_id: 'room-c', schedule_id: '' })
  })
})
