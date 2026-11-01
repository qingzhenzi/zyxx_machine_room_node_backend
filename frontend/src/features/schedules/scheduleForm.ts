import type { NewExceptionSlot, NewWeeklySlot, Schedule, WeeklySlot } from '@/api/schema'
import { WEEKDAY_LABELS, weekdayLabel } from '@/utils/format'

/** 课表新建/编辑表单字段（与 api 字段对齐，提交前再收窄为 Create/Update 请求体）。 */
export interface ScheduleFormValue {
  room_id: string
  name: string
  start_date: string
  end_date: string
  is_active: boolean
}

export function emptyScheduleForm(fallbackDate = ''): ScheduleFormValue {
  return {
    room_id: '',
    name: '',
    start_date: fallbackDate,
    end_date: fallbackDate,
    is_active: true,
  }
}

export function scheduleToForm(schedule: Schedule): ScheduleFormValue {
  return {
    room_id: schedule.room_id,
    name: schedule.name,
    start_date: schedule.start_date,
    end_date: schedule.end_date,
    is_active: schedule.is_active,
  }
}

/** 校验课表表单；`create` 时房间必填，编辑时房间不可变。 */
export function validateScheduleForm(
  value: ScheduleFormValue,
  mode: 'create' | 'edit',
): string | null {
  if (mode === 'create' && !value.room_id) return '请选择房间'
  if (!value.name.trim()) return '请填写课表名称'
  if (!value.start_date) return '请选择开始日期'
  if (!value.end_date) return '请选择结束日期'
  if (value.end_date < value.start_date) return '结束日期不能早于开始日期'
  return null
}

/** 周模板 / 单日覆盖共用的时段编辑行。 */
export interface SlotDraft {
  day_of_week: number
  start_time: string
  end_time: string
  course_name: string
  teacher: string
  note: string
}

export const WEEKDAY_OPTIONS = WEEKDAY_LABELS.map((label, index) => ({
  value: index + 1,
  label,
}))

export function emptySlotDraft(dayOfWeek = 1): SlotDraft {
  return {
    day_of_week: dayOfWeek,
    start_time: '09:00',
    end_time: '10:00',
    course_name: '',
    teacher: '',
    note: '',
  }
}

export function weeklySlotToDraft(slot: WeeklySlot): SlotDraft {
  return {
    day_of_week: slot.day_of_week ?? 1,
    start_time: slot.start_time ?? '',
    end_time: slot.end_time ?? '',
    course_name: slot.course_name ?? '',
    teacher: slot.teacher ?? '',
    note: slot.note ?? '',
  }
}

function draftDayLabel(day: number): string {
  return weekdayLabel(day)
}

/**
 * 校验时段编辑行。
 *
 * `requireDay=false` 用于单日覆盖：所有行都属于同一天，不再校验 `day_of_week`。
 * 返回首个错误文案；全部通过返回 `null`。
 */
export function validateSlotDrafts(
  rows: SlotDraft[],
  options: { requireDay?: boolean } = {},
): string | null {
  const requireDay = options.requireDay ?? true
  for (const [index, row] of rows.entries()) {
    const position = requireDay ? `第 ${index + 1} 行` : `第 ${index + 1} 个时段`
    if (
      requireDay &&
      (!Number.isInteger(row.day_of_week) || row.day_of_week < 1 || row.day_of_week > 7)
    ) {
      return `${position}请选择星期`
    }
    if (!row.start_time || !row.end_time) return `${position}请填写开始与结束时间`
    if (row.start_time >= row.end_time) return `${position}结束时间必须晚于开始时间`
    if (!row.course_name.trim()) return `${position}请填写课程名称`
  }

  const grouped = new Map<number, SlotDraft[]>()
  for (const row of rows) {
    const day = requireDay ? row.day_of_week : 1
    const list = grouped.get(day) ?? []
    list.push(row)
    grouped.set(day, list)
  }
  for (const [day, list] of grouped) {
    const sorted = [...list].sort((left, right) => left.start_time.localeCompare(right.start_time))
    for (let index = 1; index < sorted.length; index += 1) {
      const previous = sorted[index - 1]
      const current = sorted[index]
      if (!previous || !current) continue
      if (current.start_time < previous.end_time) {
        return requireDay ? `${draftDayLabel(day)} 的时段存在重叠` : '当天覆盖时段存在重叠'
      }
    }
  }
  return null
}

export function toNewWeeklySlots(rows: SlotDraft[]): NewWeeklySlot[] {
  return rows.map((row) => ({
    day_of_week: row.day_of_week,
    start_time: row.start_time,
    end_time: row.end_time,
    course_name: row.course_name.trim(),
    teacher: row.teacher.trim(),
    note: row.note.trim(),
  }))
}

export function toNewExceptionSlots(rows: SlotDraft[]): NewExceptionSlot[] {
  return rows.map((row) => ({
    start_time: row.start_time,
    end_time: row.end_time,
    course_name: row.course_name.trim(),
    teacher: row.teacher.trim(),
    note: row.note.trim(),
  }))
}

/** 本地时间 ISO 串（`2026-11-02T09:00:00+08:00`）-> 钟面 `09:00`，用于展示房间本地时间。 */
export function wallClock(iso: string | null | undefined): string {
  if (!iso) return '—'
  const match = /T(\d{2}:\d{2})/.exec(iso)
  return match?.[1] ?? '—'
}

/** `YYYY-MM-DD` -> 周几文案（后端 day_of_week 1=周一）。 */
export function weekdayOfDate(value: string | null | undefined): string {
  if (!value) return '—'
  const date = new Date(`${value}T00:00:00`)
  if (Number.isNaN(date.getTime())) return '—'
  const day = date.getDay() === 0 ? 7 : date.getDay()
  return weekdayLabel(day)
}

/** 覆盖日期排序用（后端已按日期排序，前端兜底）。 */
export function compareDate(left: string | undefined, right: string | undefined): number {
  return (left ?? '').localeCompare(right ?? '')
}
