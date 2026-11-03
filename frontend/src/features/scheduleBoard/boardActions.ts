/**
 * 排课工作台编辑动作的纯逻辑（不依赖 Vue / api）。
 *
 * 编辑语义（v1，见 docs/design/schedule-board-implementation-contract.md）：
 * 新建 / 编辑 / 删除 / 拖拽都会落到某张课表的 `PUT /weekly-slots`（全量替换）+ `POST /rebuild`。
 * 因此这里把「用户操作」翻译成「需要替换的课表 + 新的完整 slots 列表」，
 * 页面只负责依次调用接口 + 刷新，失败兜底。
 *
 * v1.5 单日调整（§5.1）：B 时间轴把用户操作翻译成「某课表某天的完整有效时段列表」，
 * 再与周模板比较，决定删除当天覆盖（恢复）/ 整天停课（cancel）/ 整套替换（replace）。
 * 计划函数只产出数据，不直接调接口；跨房间移动会产出源/目标两条计划。
 */
import type { NewExceptionSlot, NewWeeklySlot } from '@/api/schema'

import {
  BOARD_PERIODS,
  addDays,
  periodIndexOf,
  sortRooms,
  weekdayName,
  type BoardBlock,
  type BoardPayload,
  type BoardRoom,
  type BoardSchedule,
  type BoardWeeklySlot,
} from './boardTypes'

/** 新建/编辑弹窗的表单值（房间用实际房间 id，保存时换算成调课 room_id）。 */
export interface BoardSlotFormValue {
  course_name: string
  teacher: string
  room_id: string
  day_of_week: number
  start_time: string
  end_time: string
  note: string
}

/** 一次需要落库的课表替换（全量 slots）。 */
export interface SlotReplacePlan {
  schedule_id: string
  slots: NewWeeklySlot[]
}

/**
 * 单日调整里「当天的一节课」：`room_id` 与周模板语义一致
 * （`null` = 课表所属房间；非空 = 调课到其它房间），`slot_id` 来自周模板或当日覆盖。
 */
export interface BoardDaySlot {
  start_time: string
  end_time: string
  course_name: string
  teacher: string
  room_id: string | null
  note: string
  slot_id?: string
}

/** 一次单日调整需要落库的动作：删除覆盖 / 整天停课 / 整套替换。 */
export interface ExceptionPlan {
  schedule_id: string
  date: string
  action: 'delete' | 'cancel' | 'replace'
  slots: NewExceptionSlot[]
}

const TIME_PATTERN = /^([01]\d|2[0-3]):[0-5]\d$/

export function minutesOf(time: string): number {
  const [hour, minute] = time.split(':').map(Number)
  return (hour ?? 0) * 60 + (minute ?? 0)
}

/** 分钟数 -> `HH:MM`（跨天自动回绕，保证生成合法时间）。 */
export function formatMinutes(total: number): string {
  const clamped = ((Math.round(total) % 1440) + 1440) % 1440
  const hour = Math.floor(clamped / 60)
  const minute = clamped % 60
  return `${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`
}

export function addMinutes(time: string, delta: number): string {
  return formatMinutes(minutesOf(time) + delta)
}

export function durationMinutes(start: string, end: string): number {
  return minutesOf(end) - minutesOf(start)
}

export function isTimeString(value: string): boolean {
  return TIME_PATTERN.test(value)
}

export function validateSlotForm(value: BoardSlotFormValue): string {
  if (!value.course_name.trim()) return '请填写课程名称'
  if (!value.room_id) return '请选择房间'
  if (!Number.isInteger(value.day_of_week) || value.day_of_week < 1 || value.day_of_week > 7) {
    return '星期取值需在 1–7 之间'
  }
  if (!isTimeString(value.start_time) || !isTimeString(value.end_time)) {
    return '时间格式需为 HH:MM'
  }
  if (minutesOf(value.start_time) >= minutesOf(value.end_time)) {
    return '开始时间必须早于结束时间'
  }
  return ''
}

export function emptySlotForm(): BoardSlotFormValue {
  const period = BOARD_PERIODS[0]!
  return {
    course_name: '',
    teacher: '',
    room_id: '',
    day_of_week: 1,
    start_time: period.start,
    end_time: period.end,
    note: '',
  }
}

export function scheduleOfRoom(payload: BoardPayload | null, roomId: string): BoardSchedule | null {
  if (!payload || !roomId) return null
  const candidates = payload.schedules
    .filter((schedule) => schedule.room_id === roomId && schedule.is_active)
    .sort(
      (left, right) =>
        left.start_date.localeCompare(right.start_date) || left.id.localeCompare(right.id),
    )
  return candidates[0] ?? null
}

export function scheduleById(
  payload: BoardPayload | null,
  scheduleId: string,
): BoardSchedule | null {
  return payload?.schedules.find((schedule) => schedule.id === scheduleId) ?? null
}

export function weeklySlotsOf(payload: BoardPayload | null, scheduleId: string): BoardWeeklySlot[] {
  if (!payload) return []
  return payload.weekly_slots.filter((slot) => slot.schedule_id === scheduleId)
}

/** 周模板 -> NewWeeklySlot（保留未改动课块的字段，避免全量替换时丢数据）。 */
export function slotToNewWeeklySlot(slot: BoardWeeklySlot): NewWeeklySlot {
  return {
    day_of_week: slot.day_of_week,
    start_time: slot.start_time,
    end_time: slot.end_time,
    course_name: slot.course_name,
    teacher: slot.teacher ?? '',
    room_id: slot.room_id ?? null,
    note: slot.note ?? '',
  }
}

/**
 * 表单 -> NewWeeklySlot。
 * 房间等于课表所属房间时写 `null`（不产生调课标记），否则写目标房间（调课）。
 */
export function formToNewWeeklySlot(
  value: BoardSlotFormValue,
  scheduleRoomId: string | null,
): NewWeeklySlot {
  return {
    day_of_week: value.day_of_week,
    start_time: value.start_time,
    end_time: value.end_time,
    course_name: value.course_name.trim(),
    teacher: value.teacher.trim(),
    room_id: scheduleRoomId && value.room_id === scheduleRoomId ? null : value.room_id || null,
    note: value.note.trim(),
  }
}

/** 课块 -> 编辑表单（编辑时房间可改 = 调课）。 */
export function formValueOfBlock(block: BoardBlock): BoardSlotFormValue {
  return {
    course_name: block.course_name,
    teacher: block.teacher,
    room_id: block.room_id,
    day_of_week: block.day_of_week,
    start_time: block.start_time,
    end_time: block.end_time,
    note: block.note,
  }
}

/** 拖拽/改时间 -> 表单（保留课程/教师/备注，只改房间与时段）。 */
export function moveFormOfBlock(
  block: BoardBlock,
  target: {
    room_id?: string
    day_of_week?: number
    start_time: string
    end_time: string
  },
): BoardSlotFormValue {
  return {
    course_name: block.course_name,
    teacher: block.teacher,
    room_id: target.room_id ?? block.room_id,
    day_of_week: target.day_of_week ?? block.day_of_week,
    start_time: target.start_time,
    end_time: target.end_time,
    note: block.note,
  }
}

/** 新建：目标课表全量 slots + 一条新 slot。 */
export function planCreateSlot(
  payload: BoardPayload | null,
  schedule: BoardSchedule,
  value: BoardSlotFormValue,
): SlotReplacePlan[] {
  const slots = weeklySlotsOf(payload, schedule.id).map(slotToNewWeeklySlot)
  slots.push(formToNewWeeklySlot(value, schedule.room_id))
  return [{ schedule_id: schedule.id, slots }]
}

/**
 * 编辑：同课表直接替换对应 slot；换房间 = 源课表删一条 + 目标课表加一条。
 * 目标课表由调用方先 resolve（ensure 后 load 过），保证 payload 里有它的现有 slots。
 */
export function planUpdateSlot(
  payload: BoardPayload | null,
  sourceScheduleId: string,
  slotId: string,
  targetSchedule: BoardSchedule,
  value: BoardSlotFormValue,
): SlotReplacePlan[] {
  const sourceSlots = weeklySlotsOf(payload, sourceScheduleId)
  const withoutSource = sourceSlots.filter((slot) => slot.id !== slotId).map(slotToNewWeeklySlot)
  const next = formToNewWeeklySlot(value, targetSchedule.room_id)

  if (sourceScheduleId === targetSchedule.id) {
    return [{ schedule_id: sourceScheduleId, slots: [...withoutSource, next] }]
  }

  const targetSlots = weeklySlotsOf(payload, targetSchedule.id).map(slotToNewWeeklySlot)
  return [
    { schedule_id: sourceScheduleId, slots: withoutSource },
    { schedule_id: targetSchedule.id, slots: [...targetSlots, next] },
  ]
}

/** 删除：过滤掉目标 slot 后全量替换。 */
export function planDeleteSlot(
  payload: BoardPayload | null,
  scheduleId: string,
  slotId: string,
): SlotReplacePlan[] {
  return [
    {
      schedule_id: scheduleId,
      slots: weeklySlotsOf(payload, scheduleId)
        .filter((slot) => slot.id !== slotId)
        .map(slotToNewWeeklySlot),
    },
  ]
}

/** 无课表房间补默认课表时的有效期（与契约一致：本周 + 180 天）。 */
export function defaultScheduleRange(weekStart: string): { start_date: string; end_date: string } {
  return { start_date: weekStart, end_date: addDays(weekStart, 180) }
}

/** 目标节次拖拽落点：起点对齐目标节次开始，时长保持不变（允许溢出节次）。 */
export function periodDropRange(
  periodIndex: number,
  duration: number,
): { start_time: string; end_time: string } {
  const period = BOARD_PERIODS[periodIndex] ?? BOARD_PERIODS[0]!
  return { start_time: period.start, end_time: addMinutes(period.start, duration) }
}

// ---------------------------------------------------------------------------
// v1.5 单日调整（写 schedule_exceptions；见契约 §5.1）
// ---------------------------------------------------------------------------

const MAX_DAY_SLOTS = 100

function sortDaySlots(slots: BoardDaySlot[]): BoardDaySlot[] {
  return [...slots].sort(
    (left, right) =>
      left.start_time.localeCompare(right.start_time) ||
      left.end_time.localeCompare(right.end_time),
  )
}

/** 标准化当天时段的文本字段（trim），用于和模板比较。 */
function normalizeDaySlot(slot: BoardDaySlot): BoardDaySlot {
  return {
    start_time: slot.start_time,
    end_time: slot.end_time,
    course_name: slot.course_name.trim(),
    teacher: slot.teacher.trim(),
    room_id: slot.room_id || null,
    note: slot.note.trim(),
  }
}

function sameDaySlots(left: BoardDaySlot[], right: BoardDaySlot[]): boolean {
  if (left.length !== right.length) return false
  for (let index = 0; index < left.length; index += 1) {
    const a = normalizeDaySlot(left[index]!)
    const b = normalizeDaySlot(right[index]!)
    if (
      a.start_time !== b.start_time ||
      a.end_time !== b.end_time ||
      a.course_name !== b.course_name ||
      a.teacher !== b.teacher ||
      a.room_id !== b.room_id ||
      a.note !== b.note
    ) {
      return false
    }
  }
  return true
}

function daySlotToNewExceptionSlot(slot: BoardDaySlot): NewExceptionSlot {
  return {
    start_time: slot.start_time,
    end_time: slot.end_time,
    course_name: slot.course_name.trim(),
    teacher: slot.teacher.trim(),
    room_id: slot.room_id || null,
    note: slot.note.trim(),
  }
}

/** 周模板当天时段（按 start_time 排序，带模板 slot id）。 */
export function templateDaySlotsOf(
  payload: BoardPayload | null,
  scheduleId: string,
  dayOfWeek: number,
): BoardDaySlot[] {
  if (!payload) return []
  return sortDaySlots(
    payload.weekly_slots
      .filter((slot) => slot.schedule_id === scheduleId && slot.day_of_week === dayOfWeek)
      .map((slot) => ({
        start_time: slot.start_time,
        end_time: slot.end_time,
        course_name: slot.course_name ?? '',
        teacher: slot.teacher ?? '',
        room_id: slot.room_id ?? null,
        note: slot.note ?? '',
        slot_id: slot.id,
      })),
  )
}

/** 当天有效时段：有单日覆盖时以覆盖为准（cancel = []，replace = 覆盖时段），否则取周模板。 */
export function effectiveDaySlotsOf(
  payload: BoardPayload | null,
  scheduleId: string,
  date: string,
  dayOfWeek: number,
): BoardDaySlot[] {
  const exception = payload?.exceptions.find(
    (item) => item.schedule_id === scheduleId && item.exception_date === date,
  )
  if (exception) {
    if (exception.kind === 'cancel') return []
    return sortDaySlots(
      (exception.slots ?? []).map((slot) => ({
        start_time: slot.start_time,
        end_time: slot.end_time,
        course_name: slot.course_name ?? '',
        teacher: slot.teacher ?? '',
        room_id: slot.room_id ?? null,
        note: slot.note ?? '',
        slot_id: slot.id,
      })),
    )
  }
  return templateDaySlotsOf(payload, scheduleId, dayOfWeek)
}

/**
 * 表单 -> 当天时段。
 * 房间等于课表所属房间时写 `null`（不产生调课标记），否则写目标房间（调课）。
 */
export function formToBoardDaySlot(
  value: BoardSlotFormValue,
  scheduleRoomId: string | null,
): BoardDaySlot {
  return {
    start_time: value.start_time,
    end_time: value.end_time,
    course_name: value.course_name.trim(),
    teacher: value.teacher.trim(),
    room_id: scheduleRoomId && value.room_id === scheduleRoomId ? null : value.room_id || null,
    note: value.note.trim(),
  }
}

/**
 * 校验当天时段列表：时间格式 / 开始早于结束 / 互不重叠 / 最多 100 条。
 * 返回错误文案；合法时返回空串。后端也会校验，这里先拦以避免 400。
 */
export function validateDaySlots(nextSlots: BoardDaySlot[]): string {
  if (nextSlots.length > MAX_DAY_SLOTS) {
    return `当天最多 ${MAX_DAY_SLOTS} 条覆盖时段`
  }
  for (const slot of nextSlots) {
    if (!isTimeString(slot.start_time) || !isTimeString(slot.end_time)) {
      return '时间格式需为 HH:MM'
    }
    if (minutesOf(slot.start_time) >= minutesOf(slot.end_time)) {
      return '开始时间必须早于结束时间'
    }
  }
  const sorted = sortDaySlots(nextSlots)
  for (let index = 1; index < sorted.length; index += 1) {
    const previous = sorted[index - 1]!
    const current = sorted[index]!
    if (current.start_time < previous.end_time) {
      return `当天时段不能重叠：${previous.start_time}–${previous.end_time} 与 ${current.start_time}–${current.end_time}`
    }
  }
  return ''
}

/**
 * 计算某课表某天的单日覆盖计划：
 * - 与周模板当天完全一致 → `delete`（删除当天覆盖 = 恢复模板）；
 * - 为空 → `cancel`（整天停课）；
 * - 否则 → `replace`（整套当天时段）。
 */
export function planDayException(
  payload: BoardPayload | null,
  scheduleId: string,
  date: string,
  dayOfWeek: number,
  nextSlots: BoardDaySlot[],
): ExceptionPlan {
  const next = sortDaySlots(nextSlots)
  const template = templateDaySlotsOf(payload, scheduleId, dayOfWeek)
  if (sameDaySlots(next, template)) {
    return { schedule_id: scheduleId, date, action: 'delete', slots: [] }
  }
  if (!next.length) {
    return { schedule_id: scheduleId, date, action: 'cancel', slots: [] }
  }
  return {
    schedule_id: scheduleId,
    date,
    action: 'replace',
    slots: next.map(daySlotToNewExceptionSlot),
  }
}

function scheduleRoomIdOf(payload: BoardPayload | null, scheduleId: string): string | null {
  return payload?.schedules.find((schedule) => schedule.id === scheduleId)?.room_id ?? null
}

/**
 * 在当天有效时段里定位课块：优先 `slot_id`（模板 slot id / 覆盖 slot id），
 * 回退到「时间 + 课程 + 教师 + 房间」的完整匹配。
 */
function matchDaySlotIndex(
  payload: BoardPayload | null,
  scheduleId: string,
  block: BoardBlock,
  slots: BoardDaySlot[],
): number {
  if (block.slot_id) {
    const byId = slots.findIndex((slot) => slot.slot_id === block.slot_id)
    if (byId >= 0) return byId
  }
  const scheduleRoomId = scheduleRoomIdOf(payload, scheduleId)
  return slots.findIndex((slot) => {
    const slotRoomId = slot.room_id ?? scheduleRoomId
    return (
      slot.start_time === block.start_time &&
      slot.end_time === block.end_time &&
      slot.course_name.trim() === block.course_name.trim() &&
      slot.teacher.trim() === block.teacher.trim() &&
      slotRoomId === block.room_id
    )
  })
}

/** 课块 -> 当天时段（移动/复制时保留课程、教师、备注，只改房间与时间）。 */
function daySlotFromBlock(
  block: BoardBlock,
  startTime: string,
  endTime: string,
  targetRoomId: string,
  targetScheduleRoomId: string | null,
): BoardDaySlot {
  return {
    start_time: startTime,
    end_time: endTime,
    course_name: block.course_name.trim(),
    teacher: block.teacher.trim(),
    room_id: targetScheduleRoomId && targetRoomId === targetScheduleRoomId ? null : targetRoomId,
    note: block.note.trim(),
  }
}

/** 本次停课：从当天有效时段里移除该课块；移除后为空则整天 cancel，否则 replace。 */
export function planOccurrenceCancel(
  payload: BoardPayload | null,
  block: BoardBlock,
): ExceptionPlan[] {
  const slots = effectiveDaySlotsOf(payload, block.schedule_id, block.date, block.day_of_week)
  const index = matchDaySlotIndex(payload, block.schedule_id, block, slots)
  if (index < 0) return []
  return [
    planDayException(
      payload,
      block.schedule_id,
      block.date,
      block.day_of_week,
      slots.filter((_, slotIndex) => slotIndex !== index),
    ),
  ]
}

function sameCourseTeacher(slot: BoardDaySlot, block: BoardBlock): boolean {
  return (
    slot.course_name.trim() === block.course_name.trim() &&
    slot.teacher.trim() === block.teacher.trim()
  )
}

/**
 * 跨房间「单日移动」在源课表当天留下了一条排除该课的覆盖；
 * 恢复目标课表的这一次课时，需要同时把源课表当天的这节课加回去。
 *
 * 识别方式：同一日期下另一课表存在覆盖、当天有效时段缺少该课、而周模板当天包含该课，
 * 且模板里这节课的房间与当前课块房间不同（确保确实是调出而非同房间改时间）。
 */
function restoreMovedSources(payload: BoardPayload | null, block: BoardBlock): ExceptionPlan[] {
  if (!payload) return []
  const plans: ExceptionPlan[] = []
  for (const schedule of payload.schedules) {
    if (!schedule.is_active || schedule.id === block.schedule_id) continue
    const exception = payload.exceptions.find(
      (item) => item.schedule_id === schedule.id && item.exception_date === block.date,
    )
    if (!exception) continue
    const template = templateDaySlotsOf(payload, schedule.id, block.day_of_week)
    const matched = template.find((slot) => sameCourseTeacher(slot, block))
    if (!matched) continue
    const templateRoom = matched.room_id ?? schedule.room_id
    if (templateRoom === block.room_id) continue
    const effective = effectiveDaySlotsOf(payload, schedule.id, block.date, block.day_of_week)
    if (effective.some((slot) => sameCourseTeacher(slot, block))) continue
    plans.push(
      planDayException(payload, schedule.id, block.date, block.day_of_week, [
        ...effective,
        matched,
      ]),
    )
  }
  return plans
}

/**
 * 恢复本次：
 * - 当前课表这一天恢复为周模板；
 * - 若是「跨房间单日调课」产生的目标课块，同时把源课表当天的这节课加回原位。
 */
export function planOccurrenceRestore(
  payload: BoardPayload | null,
  block: BoardBlock,
): ExceptionPlan[] {
  const exception = payload?.exceptions.find(
    (item) => item.schedule_id === block.schedule_id && item.exception_date === block.date,
  )
  if (!exception) return []
  return [
    planDayException(
      payload,
      block.schedule_id,
      block.date,
      block.day_of_week,
      templateDaySlotsOf(payload, block.schedule_id, block.day_of_week),
    ),
    ...restoreMovedSources(payload, block),
  ]
}

/**
 * 单日移动：只调整 `block.date` 这一天的这一节课。
 * - 同课表（同房间改时间）：一条计划；
 * - 跨课表（换房间）：源课表移除该节、目标课表加入该节，两条计划（目标房间 room 语义 = null）。
 */
export function planOccurrenceMove(
  payload: BoardPayload | null,
  block: BoardBlock,
  target: { room_id: string; start_time: string; end_time: string },
  targetSchedule: BoardSchedule,
): ExceptionPlan[] {
  if (!payload) return []
  const date = block.date
  const dayOfWeek = block.day_of_week
  const sourceSlots = effectiveDaySlotsOf(payload, block.schedule_id, date, dayOfWeek)
  const index = matchDaySlotIndex(payload, block.schedule_id, block, sourceSlots)
  if (index < 0) return []

  if (targetSchedule.id === block.schedule_id) {
    const next = [
      ...sourceSlots.filter((_, slotIndex) => slotIndex !== index),
      daySlotFromBlock(
        block,
        target.start_time,
        target.end_time,
        target.room_id,
        targetSchedule.room_id,
      ),
    ]
    return [planDayException(payload, block.schedule_id, date, dayOfWeek, next)]
  }

  const sourceNext = sourceSlots.filter((_, slotIndex) => slotIndex !== index)
  const targetSlots = effectiveDaySlotsOf(payload, targetSchedule.id, date, dayOfWeek)
  const targetNext = [
    ...targetSlots,
    daySlotFromBlock(
      block,
      target.start_time,
      target.end_time,
      target.room_id,
      targetSchedule.room_id,
    ),
  ]
  return [
    planDayException(payload, block.schedule_id, date, dayOfWeek, sourceNext),
    planDayException(payload, targetSchedule.id, date, dayOfWeek, targetNext),
  ]
}

/** 新建本次课程：当天有效时段 + 新时段，再按模板比较生成计划。 */
export function planOccurrenceCreate(
  payload: BoardPayload | null,
  schedule: BoardSchedule,
  date: string,
  dayOfWeek: number,
  value: BoardSlotFormValue,
): ExceptionPlan[] {
  const next = [
    ...effectiveDaySlotsOf(payload, schedule.id, date, dayOfWeek),
    formToBoardDaySlot(value, schedule.room_id),
  ]
  return [planDayException(payload, schedule.id, date, dayOfWeek, next)]
}

// ---------------------------------------------------------------------------
// v1.6 撤销/重做快照（见契约 §5.2）
// ---------------------------------------------------------------------------

/** 撤销/重做用：某张课表操作前的全量周模板。 */
export interface TemplateSnapshot {
  schedule_id: string
  slots: NewWeeklySlot[]
}

/** 撤销/重做用：某课表某天操作前的单日覆盖；`kind = null` 表示当天没有覆盖。 */
export interface ExceptionSnapshot {
  schedule_id: string
  date: string
  kind: 'cancel' | 'replace' | null
  note: string
  slots: NewExceptionSlot[]
}

/** 单日计划引用的 `(schedule_id, date)` 去重列表（撤销前取快照用）。 */
export function exceptionPlanRefs(
  plans: ExceptionPlan[],
): Array<{ schedule_id: string; date: string }> {
  const seen = new Set<string>()
  const refs: Array<{ schedule_id: string; date: string }> = []
  for (const plan of plans) {
    const key = `${plan.schedule_id}|${plan.date}`
    if (seen.has(key)) continue
    seen.add(key)
    refs.push({ schedule_id: plan.schedule_id, date: plan.date })
  }
  return refs
}

/** 取各课表当前的全量周模板（按传入顺序去重）。 */
export function snapshotTemplates(
  payload: BoardPayload | null,
  scheduleIds: string[],
): TemplateSnapshot[] {
  const seen = new Set<string>()
  const snapshots: TemplateSnapshot[] = []
  for (const scheduleId of scheduleIds) {
    if (seen.has(scheduleId)) continue
    seen.add(scheduleId)
    snapshots.push({
      schedule_id: scheduleId,
      slots: weeklySlotsOf(payload, scheduleId).map(slotToNewWeeklySlot),
    })
  }
  return snapshots
}

/** 取各课表当天已有的单日覆盖（按 schedule_id + date 去重；无覆盖时 kind = null）。 */
export function snapshotExceptions(
  payload: BoardPayload | null,
  refs: Array<{ schedule_id: string; date: string }>,
): ExceptionSnapshot[] {
  const seen = new Set<string>()
  const snapshots: ExceptionSnapshot[] = []
  for (const ref of refs) {
    const key = `${ref.schedule_id}|${ref.date}`
    if (seen.has(key)) continue
    seen.add(key)
    const exception = payload?.exceptions.find(
      (item) => item.schedule_id === ref.schedule_id && item.exception_date === ref.date,
    )
    snapshots.push({
      schedule_id: ref.schedule_id,
      date: ref.date,
      kind: exception?.kind ?? null,
      note: exception?.note ?? '',
      slots: (exception?.slots ?? []).map((slot) => ({
        start_time: slot.start_time,
        end_time: slot.end_time,
        course_name: slot.course_name,
        teacher: slot.teacher,
        room_id: slot.room_id,
        note: slot.note,
      })),
    })
  }
  return snapshots
}

// ---------------------------------------------------------------------------
// v1.7 C 矩阵框选批量应用/清除（见契约 §5.3）
// ---------------------------------------------------------------------------

/** C 编排矩阵里选中的一个格子：房间 + 星期 + 节次下标（与 periodPlacement 一致）。 */
export interface MatrixCellRef {
  room_id: string
  day_of_week: number
  period_index: number
}

export type BatchFillMode = 'empty' | 'overwrite'

/** 批量应用的表单值（房间由选中格子决定，时段固定取 BOARD_PERIODS）。 */
export interface BatchFillValue {
  course_name: string
  teacher: string
  note: string
}

export interface BatchFillRoomResult {
  room_id: string
  /** 该房间的课表 id；空串表示房间还没有课表，执行前会先 ensure。 */
  schedule_id: string
  created: number
  skipped: number
}

export interface BatchFillResult {
  plans: SlotReplacePlan[]
  created: number
  skipped: number
  perRoom: BatchFillRoomResult[]
  errors: string[]
}

export interface BatchClearRoomResult {
  room_id: string
  schedule_id: string
  cleared: number
}

export interface BatchClearResult {
  plans: SlotReplacePlan[]
  cleared: number
  perRoom: BatchClearRoomResult[]
  errors: string[]
}

function dedupeMatrixCells(cells: MatrixCellRef[]): MatrixCellRef[] {
  const seen = new Set<string>()
  const result: MatrixCellRef[] = []
  for (const cell of cells) {
    if (!cell.room_id) continue
    if (!Number.isInteger(cell.day_of_week) || cell.day_of_week < 1 || cell.day_of_week > 7)
      continue
    if (
      !Number.isInteger(cell.period_index) ||
      cell.period_index < 0 ||
      cell.period_index >= BOARD_PERIODS.length
    ) {
      continue
    }
    const key = `${cell.room_id}|${cell.day_of_week}|${cell.period_index}`
    if (seen.has(key)) continue
    seen.add(key)
    result.push({ ...cell })
  }
  return result
}

function groupCellsByRoom(cells: MatrixCellRef[]): Map<string, MatrixCellRef[]> {
  const grouped = new Map<string, MatrixCellRef[]>()
  for (const cell of dedupeMatrixCells(cells)) {
    const list = grouped.get(cell.room_id) ?? []
    list.push(cell)
    grouped.set(cell.room_id, list)
  }
  for (const list of grouped.values()) {
    list.sort(
      (left, right) =>
        left.day_of_week - right.day_of_week || left.period_index - right.period_index,
    )
  }
  return grouped
}

function roomCodeOf(payload: BoardPayload | null, roomId: string): string {
  return payload?.rooms.find((room) => room.id === roomId)?.code ?? roomId.slice(0, 8)
}

/** 只校验批量涉及到的星期：重叠 / 同一天超过 100 条。 */
function batchDaySlotsError(slots: NewWeeklySlot[], days: Set<number>): string {
  for (const day of days) {
    const daySlots = slots.filter((slot) => slot.day_of_week === day)
    if (!daySlots.length) continue
    const message = validateDaySlots(
      daySlots.map((slot) => ({
        start_time: slot.start_time,
        end_time: slot.end_time,
        course_name: slot.course_name,
        teacher: slot.teacher ?? '',
        room_id: slot.room_id ?? null,
        note: slot.note ?? '',
      })),
    )
    if (message) return `${weekdayName(day)} ${message}`
  }
  return ''
}

/**
 * 批量应用计划：按选中格子所在房间分组，每组生成该课表的全量 slots 替换。
 * - 时段固定取 `BOARD_PERIODS[period_index]`；
 * - 占用判定 = 该课表同一天且 `periodIndexOf(start_time) === period_index`；
 * - `empty` 模式跳过占用格；`overwrite` 模式先移除占用 slot 再追加；
 * - 房间无课表时不产出 plan（页面先 ensure 后重新计划），但按「空格子」计入 created 预览。
 */
export function planBatchFill(
  payload: BoardPayload | null,
  cells: MatrixCellRef[],
  value: BatchFillValue,
  mode: BatchFillMode,
): BatchFillResult {
  const result: BatchFillResult = {
    plans: [],
    created: 0,
    skipped: 0,
    perRoom: [],
    errors: [],
  }
  const grouped = groupCellsByRoom(cells)
  for (const [roomId, roomCells] of grouped) {
    const schedule = scheduleOfRoom(payload, roomId)
    if (!schedule) {
      const created = roomCells.length
      result.created += created
      result.perRoom.push({ room_id: roomId, schedule_id: '', created, skipped: 0 })
      continue
    }

    let slots = weeklySlotsOf(payload, schedule.id).map(slotToNewWeeklySlot)
    let created = 0
    let skipped = 0
    const touchedDays = new Set<number>()
    for (const cell of roomCells) {
      const period = BOARD_PERIODS[cell.period_index]
      if (!period) continue
      touchedDays.add(cell.day_of_week)
      const occupied = slots.filter(
        (slot) =>
          slot.day_of_week === cell.day_of_week &&
          periodIndexOf(slot.start_time) === cell.period_index,
      )
      if (mode === 'empty') {
        if (occupied.length) {
          skipped += 1
          continue
        }
      } else if (occupied.length) {
        const occupiedSet = new Set(occupied)
        slots = slots.filter((slot) => !occupiedSet.has(slot))
      }
      slots.push({
        day_of_week: cell.day_of_week,
        start_time: period.start,
        end_time: period.end,
        course_name: value.course_name.trim(),
        teacher: value.teacher.trim(),
        room_id: null,
        note: value.note.trim(),
      })
      created += 1
    }

    const error = batchDaySlotsError(slots, touchedDays)
    if (error) result.errors.push(`${roomCodeOf(payload, roomId)} ${error}`)
    result.created += created
    result.skipped += skipped
    result.perRoom.push({ room_id: roomId, schedule_id: schedule.id, created, skipped })
    if (created > 0) result.plans.push({ schedule_id: schedule.id, slots })
  }
  return result
}

/** 批量清除：移除选中格子对应「同课表 + 同星期 + 同节次」的全部周模板 slot。 */
export function planBatchClear(
  payload: BoardPayload | null,
  cells: MatrixCellRef[],
): BatchClearResult {
  const result: BatchClearResult = { plans: [], cleared: 0, perRoom: [], errors: [] }
  const grouped = groupCellsByRoom(cells)
  for (const [roomId, roomCells] of grouped) {
    const schedule = scheduleOfRoom(payload, roomId)
    if (!schedule) continue
    const cellKeys = new Set(roomCells.map((cell) => `${cell.day_of_week}|${cell.period_index}`))
    const current = weeklySlotsOf(payload, schedule.id)
    const kept = current
      .filter((slot) => !cellKeys.has(`${slot.day_of_week}|${periodIndexOf(slot.start_time)}`))
      .map(slotToNewWeeklySlot)
    const cleared = current.length - kept.length
    if (!cleared) continue
    result.plans.push({ schedule_id: schedule.id, slots: kept })
    result.cleared += cleared
    result.perRoom.push({ room_id: roomId, schedule_id: schedule.id, cleared })
  }
  return result
}

// ---------------------------------------------------------------------------
// v1.8 C 矩阵矩形复制/粘贴（见契约 §5.4）
// ---------------------------------------------------------------------------

export interface MatrixClipboardSlot {
  start_time: string
  end_time: string
  course_name: string
  teacher: string
  note: string
}

/** 剪贴板里的一个源格：`row/col` 是相对矩形左上角的偏移。 */
export interface MatrixClipboardCell {
  row: number
  col: number
  slots: MatrixClipboardSlot[]
}

export interface MatrixClipboard {
  /** 排序后的 room_id（与 C 矩阵行顺序一致）。 */
  rows: string[]
  columns: Array<{ day_of_week: number; period_index: number }>
  cells: MatrixClipboardCell[]
}

export interface BatchPasteRoomResult {
  room_id: string
  /** 空串表示目标房间还没有课表，执行前会先 ensure。 */
  schedule_id: string
  created: number
  overwritten: number
}

export interface BatchPasteResult {
  plans: SlotReplacePlan[]
  created: number
  overwritten: number
  skipped: number
  perRoom: BatchPasteRoomResult[]
  errors: string[]
}

function matrixColumnIndexOf(dayOfWeek: number, periodIndex: number): number {
  return (dayOfWeek - 1) * BOARD_PERIODS.length + periodIndex
}

function matrixColumnFromIndex(index: number): { day_of_week: number; period_index: number } {
  return {
    day_of_week: Math.floor(index / BOARD_PERIODS.length) + 1,
    period_index: index % BOARD_PERIODS.length,
  }
}

/**
 * 从 C 矩阵选区生成剪贴板：只复制周模板课块（当天有单日覆盖的格子按「非模板」跳过），
 * rows/columns 记录选区矩形范围，cells 只保留有课块的源格及其相对偏移。
 */
export function buildMatrixClipboard(
  payload: BoardPayload | null,
  rooms: BoardRoom[],
  cells: MatrixCellRef[],
): MatrixClipboard {
  const orderedRooms = sortRooms(rooms)
  const roomIndexById = new Map(orderedRooms.map((room, index) => [room.id, index]))
  const deduped = dedupeMatrixCells(cells)
  if (!deduped.length) return { rows: [], columns: [], cells: [] }

  const rowIndices: number[] = []
  const columnIndices: number[] = []
  for (const cell of deduped) {
    const rowIndex = roomIndexById.get(cell.room_id)
    if (rowIndex === undefined) continue
    rowIndices.push(rowIndex)
    columnIndices.push(matrixColumnIndexOf(cell.day_of_week, cell.period_index))
  }
  if (!rowIndices.length) return { rows: [], columns: [], cells: [] }

  const rowStart = Math.min(...rowIndices)
  const rowEnd = Math.max(...rowIndices)
  const columnStart = Math.min(...columnIndices)
  const columnEnd = Math.max(...columnIndices)
  const rows = orderedRooms.slice(rowStart, rowEnd + 1).map((room) => room.id)
  const columns = Array.from({ length: columnEnd - columnStart + 1 }, (_, offset) =>
    matrixColumnFromIndex(columnStart + offset),
  )

  const clipboardCells: MatrixClipboardCell[] = []
  for (const cell of deduped) {
    const rowIndex = roomIndexById.get(cell.room_id)
    if (rowIndex === undefined) continue
    const schedule = scheduleOfRoom(payload, cell.room_id)
    if (!schedule) continue
    const date = payload?.from ? addDays(payload.from, cell.day_of_week - 1) : ''
    // 当天有单日覆盖时 C 里显示的是覆盖块（source=exception），不算模板课块。
    if (
      payload?.exceptions.some(
        (exception) => exception.schedule_id === schedule.id && exception.exception_date === date,
      )
    ) {
      continue
    }
    const slots = weeklySlotsOf(payload, schedule.id)
      .filter(
        (slot) =>
          slot.day_of_week === cell.day_of_week &&
          periodIndexOf(slot.start_time) === cell.period_index,
      )
      .map((slot) => ({
        start_time: slot.start_time,
        end_time: slot.end_time,
        course_name: slot.course_name,
        teacher: slot.teacher ?? '',
        note: slot.note ?? '',
      }))
    if (!slots.length) continue
    clipboardCells.push({
      row: rowIndex - rowStart,
      col: matrixColumnIndexOf(cell.day_of_week, cell.period_index) - columnStart,
      slots,
    })
  }

  return { rows, columns, cells: clipboardCells }
}

/**
 * 矩形粘贴计划：点击的格子作为矩形左上角锚点，按剪贴板相对偏移逐格写入目标课表。
 * - 目标格先移除同房间同 day 且 periodIndexOf 命中节次的模板 slot，再写入复制块；
 * - 时间 = 目标节次起点 + (源块起点 - 源节次起点)，时长不变；
 * - 越界（房间行 / 7 天×5 节次列）或时间平移越界 → 计入 skipped；
 * - 源格无课块的格子不在 cells 里，因此目标格不会被清空；重叠/超 100 条 → errors。
 */
export function planBatchPaste(
  payload: BoardPayload | null,
  clipboard: MatrixClipboard,
  anchorCell: MatrixCellRef,
  rooms: BoardRoom[],
): BatchPasteResult {
  const result: BatchPasteResult = {
    plans: [],
    created: 0,
    overwritten: 0,
    skipped: 0,
    perRoom: [],
    errors: [],
  }
  const orderedRooms = sortRooms(rooms)
  const anchorRow = orderedRooms.findIndex((room) => room.id === anchorCell.room_id)
  if (anchorRow < 0) {
    result.errors.push('粘贴锚点不在当前矩阵房间内')
    return result
  }
  const anchorColumn = matrixColumnIndexOf(anchorCell.day_of_week, anchorCell.period_index)
  const rowCount = orderedRooms.length
  const columnCount = 7 * BOARD_PERIODS.length

  const byRoom = new Map<
    string,
    Array<{ day_of_week: number; period_index: number; slots: MatrixClipboardSlot[] }>
  >()
  for (const cell of clipboard.cells) {
    const targetRow = anchorRow + cell.row
    const targetColumn = anchorColumn + cell.col
    if (targetRow < 0 || targetRow >= rowCount || targetColumn < 0 || targetColumn >= columnCount) {
      result.skipped += 1
      continue
    }
    const targetRoom = orderedRooms[targetRow]
    const sourceColumn = clipboard.columns[cell.col]
    if (!targetRoom || !sourceColumn) {
      result.skipped += 1
      continue
    }
    const target = matrixColumnFromIndex(targetColumn)
    const sourcePeriod = BOARD_PERIODS[sourceColumn.period_index]
    const targetPeriod = BOARD_PERIODS[target.period_index]
    if (!sourcePeriod || !targetPeriod) {
      result.skipped += 1
      continue
    }
    const list = byRoom.get(targetRoom.id) ?? []
    for (const slot of cell.slots) {
      const duration = minutesOf(slot.end_time) - minutesOf(slot.start_time)
      const offset = minutesOf(slot.start_time) - minutesOf(sourcePeriod.start)
      const start = minutesOf(targetPeriod.start) + offset
      const end = start + duration
      if (start < 0 || end > 1440 || end <= start) {
        result.skipped += 1
        continue
      }
      list.push({
        day_of_week: target.day_of_week,
        period_index: target.period_index,
        slots: [
          {
            start_time: formatMinutes(start),
            end_time: formatMinutes(end),
            course_name: slot.course_name,
            teacher: slot.teacher,
            note: slot.note,
          },
        ],
      })
    }
    byRoom.set(targetRoom.id, list)
  }

  for (const [roomId, targets] of byRoom) {
    const schedule = scheduleOfRoom(payload, roomId)
    if (!schedule) {
      const created = targets.reduce((total, target) => total + target.slots.length, 0)
      result.created += created
      result.perRoom.push({ room_id: roomId, schedule_id: '', created, overwritten: 0 })
      continue
    }

    let slots = weeklySlotsOf(payload, schedule.id).map(slotToNewWeeklySlot)
    let created = 0
    let overwritten = 0
    const touchedDays = new Set<number>()
    for (const target of targets) {
      touchedDays.add(target.day_of_week)
      const before = slots.length
      slots = slots.filter(
        (slot) =>
          !(
            slot.day_of_week === target.day_of_week &&
            periodIndexOf(slot.start_time) === target.period_index
          ),
      )
      overwritten += before - slots.length
      for (const slot of target.slots) {
        slots.push({
          day_of_week: target.day_of_week,
          start_time: slot.start_time,
          end_time: slot.end_time,
          course_name: slot.course_name,
          teacher: slot.teacher,
          room_id: null,
          note: slot.note,
        })
        created += 1
      }
    }

    const error = batchDaySlotsError(slots, touchedDays)
    if (error) result.errors.push(`${roomCodeOf(payload, roomId)} ${error}`)
    result.created += created
    result.overwritten += overwritten
    result.perRoom.push({ room_id: roomId, schedule_id: schedule.id, created, overwritten })
    if (created > 0) result.plans.push({ schedule_id: schedule.id, slots })
  }

  return result
}
