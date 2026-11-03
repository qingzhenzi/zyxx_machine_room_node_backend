/**
 * 排课工作台（/schedules）共享类型与纯函数。
 *
 * 数据来源：`GET /api/v1/admin/schedule-board?from=&to=&area_id=` 一次返回
 * 区域 / 房间 / 课表 / 周模板 / 单日覆盖；前端在此之上做「按周展开 + 冲突计算」，
 * 不依赖物化实例（occurrences），因此不受后端重建窗口影响。
 *
 * 编辑语义（v1）：
 * - A 周网格 / B 时间轴 / C 编排矩阵 中拖动、新建、编辑课块，都是改「周模板」（影响整个生效区间）；
 * - 单日覆盖（exception）在 v1 中只读展示（来源徽标「单日」），后续版本再支持单日调课。
 */
export type BoardViewMode = 'week' | 'timeline' | 'matrix'

export interface BoardDeviceInfo {
  serial: string | null
  status: string | null
  online: boolean | null
}

export interface BoardRoom {
  id: string
  area_id: string
  area_name: string
  code: string
  name: string
  capacity: number
  device_slot: string | null
  is_active: boolean
  timezone: string
  device: BoardDeviceInfo | null
}

export interface BoardSchedule {
  id: string
  room_id: string
  name: string
  start_date: string
  end_date: string
  is_active: boolean
  timezone: string
}

export interface BoardArea {
  id: string
  code: string
  name: string
  timezone: string
}

export interface BoardWeeklySlot {
  id: string
  schedule_id: string
  day_of_week: number
  start_time: string
  end_time: string
  course_name: string
  teacher: string
  room_id: string | null
  note: string
}

export interface BoardExceptionSlot {
  id: string
  start_time: string
  end_time: string
  course_name: string
  teacher: string
  room_id: string | null
  note: string
}

export interface BoardException {
  id: string
  schedule_id: string
  exception_date: string
  kind: 'cancel' | 'replace'
  note: string
  slots: BoardExceptionSlot[]
}

export interface BoardPayload {
  from: string
  to: string
  areas: BoardArea[]
  rooms: BoardRoom[]
  schedules: BoardSchedule[]
  weekly_slots: BoardWeeklySlot[]
  exceptions: BoardException[]
}

export interface BoardQuery {
  from: string
  to: string
  area_id?: string
}

export interface EnsureSchedulesRequest {
  room_ids: string[]
  start_date: string
  end_date: string
  name?: string
}

export interface EnsureSchedulesReply {
  created: BoardSchedule[]
  existing: BoardSchedule[]
}

export interface BoardConflict {
  kind: 'room' | 'teacher'
  severity: 'error' | 'warning'
  date: string
  message: string
  /** 冲突双方的课块 key。 */
  block_keys: string[]
}

export type BoardBlockSource = 'template' | 'exception'

/** 展开后的一节课（某一具体日期的具体课块）。 */
export interface BoardBlock {
  /** `template:<slotId>:<date>` 或 `exception:<slotId>:<date>` */
  key: string
  source: BoardBlockSource
  schedule_id: string
  slot_id: string
  room_id: string
  room_code: string
  room_name: string
  area_id: string
  area_name: string
  date: string
  day_of_week: number
  start_time: string
  end_time: string
  course_name: string
  teacher: string
  note: string
  /** 周模板通过 room_id 调课到其它房间。 */
  is_override: boolean
  /** 该块参与的全部冲突。 */
  conflicts: BoardConflict[]
}

export interface BoardPeriod {
  key: string
  label: string
  start: string
  end: string
}

/** 固定节次（与原型一致）；真实时段不落在节次内时按最近节次归档。 */
export const BOARD_PERIODS: BoardPeriod[] = [
  { key: 'p1', label: '第1-2节', start: '08:00', end: '09:40' },
  { key: 'p3', label: '第3-4节', start: '10:00', end: '11:40' },
  { key: 'p5', label: '第5-6节', start: '13:30', end: '15:10' },
  { key: 'p7', label: '第7-8节', start: '15:30', end: '17:10' },
  { key: 'p9', label: '第9-10节', start: '18:30', end: '20:10' },
]

export const DEFAULT_SCHEDULE_DAYS = 180

const WEEKDAY_NAMES = ['周一', '周二', '周三', '周四', '周五', '周六', '周日']

/** 课程色板：按课程名稳定取色（同一门课全视图同色）。 */
// 刻意避开琥珀/橙与正红色：这两种颜色留给冲突提示（教师冲突 / 房间冲突）。
const COURSE_COLORS = [
  { fg: '#3a5bd9', bg: '#eef2ff' },
  { fg: '#0f8b6c', bg: '#e9f7f1' },
  { fg: '#7a4fd6', bg: '#f2edfd' },
  { fg: '#0b7f96', bg: '#e8f6f8' },
  { fg: '#5b6b8c', bg: '#eef1f6' },
  { fg: '#a34d8a', bg: '#fbeef7' },
  { fg: '#4169a5', bg: '#edf2fa' },
  { fg: '#2f7d5c', bg: '#eaf5ef' },
]

export function courseColor(course: string): { fg: string; bg: string } {
  let hash = 0
  for (let index = 0; index < course.length; index += 1) {
    hash = (hash * 31 + course.charCodeAt(index)) >>> 0
  }
  return COURSE_COLORS[hash % COURSE_COLORS.length] ?? COURSE_COLORS[0]!
}

export function weekdayName(dayOfWeek: number): string {
  return WEEKDAY_NAMES[dayOfWeek - 1] ?? '—'
}

/** 解析 `YYYY-MM-DD` 为 UTC 日期的毫秒数（纯日期运算，不受本地时区影响）。 */
function dateValue(date: string): number {
  const [year, month, day] = date.split('-').map(Number)
  return Date.UTC(year ?? 1970, (month ?? 1) - 1, day ?? 1)
}

function dateFromValue(value: number): string {
  return new Date(value).toISOString().slice(0, 10)
}

export function addDays(date: string, days: number): string {
  return dateFromValue(dateValue(date) + days * 86_400_000)
}

export function diffDays(from: string, to: string): number {
  return Math.round((dateValue(to) - dateValue(from)) / 86_400_000)
}

export function isoWeekday(date: string): number {
  return ((new Date(dateValue(date)).getUTCDay() + 6) % 7) + 1
}

/** 返回 date 所在周的周一。 */
export function weekStartOf(date: string): string {
  return addDays(date, -(isoWeekday(date) - 1))
}

/** 返回 weekStart 起连续 7 天的日期数组。 */
export function weekDates(weekStart: string): string[] {
  return Array.from({ length: 7 }, (_, index) => addDays(weekStart, index))
}

export function formatMonthDay(date: string): string {
  return `${Number(date.slice(5, 7))}/${Number(date.slice(8, 10))}`
}

export function formatWeekRange(weekStart: string): string {
  return `${formatMonthDay(weekStart)}–${formatMonthDay(addDays(weekStart, 6))}`
}

export function isSameDate(a: string, b: string): boolean {
  return a === b
}

function timeMinutes(time: string): number {
  const [hour, minute] = time.split(':').map(Number)
  return (hour ?? 0) * 60 + (minute ?? 0)
}

/** 时段落入的节次下标；落在节次之外时取最近节次。 */
export function periodIndexOf(startTime: string): number {
  const value = timeMinutes(startTime)
  let best = 0
  let bestDistance = Number.POSITIVE_INFINITY
  for (const [index, period] of BOARD_PERIODS.entries()) {
    const start = timeMinutes(period.start)
    const end = timeMinutes(period.end)
    if (value >= start && value < end) return index
    const distance = Math.min(Math.abs(value - start), Math.abs(value - end))
    if (distance < bestDistance) {
      bestDistance = distance
      best = index
    }
  }
  return best
}

export interface PeriodPlacement {
  /** 从 0 开始的节次下标。 */
  index: number
  /** 课块在节次内的纵向比例（0~1），用于 C 矩阵内的近似定位。 */
  offset: number
}

export function periodPlacement(startTime: string): PeriodPlacement {
  const index = periodIndexOf(startTime)
  const period = BOARD_PERIODS[index]!
  const start = timeMinutes(period.start)
  const end = timeMinutes(period.end)
  const span = Math.max(1, end - start)
  const offset = Math.min(1, Math.max(0, (timeMinutes(startTime) - start) / span))
  return { index, offset }
}

interface ExpandContext {
  roomsById: Map<string, BoardRoom>
  schedulesById: Map<string, BoardSchedule>
  exceptionsByKey: Map<string, BoardException>
}

function buildContext(payload: BoardPayload): ExpandContext {
  return {
    roomsById: new Map(payload.rooms.map((room) => [room.id, room])),
    schedulesById: new Map(payload.schedules.map((schedule) => [schedule.id, schedule])),
    exceptionsByKey: new Map(
      payload.exceptions.map((exception) => [
        `${exception.schedule_id}|${exception.exception_date}`,
        exception,
      ]),
    ),
  }
}

function makeBlock(
  context: ExpandContext,
  params: {
    source: BoardBlockSource
    schedule_id: string
    slot_id: string
    room_id: string | null
    date: string
    day_of_week: number
    start_time: string
    end_time: string
    course_name: string
    teacher: string
    note: string
  },
): BoardBlock | null {
  const schedule = context.schedulesById.get(params.schedule_id)
  if (!schedule) return null
  const room = context.roomsById.get(params.room_id ?? schedule.room_id)
  if (!room) return null
  return {
    key: `${params.source === 'template' ? 'template' : 'exception'}:${params.slot_id}:${params.date}`,
    source: params.source,
    schedule_id: params.schedule_id,
    slot_id: params.slot_id,
    room_id: room.id,
    room_code: room.code,
    room_name: room.name,
    area_id: room.area_id,
    area_name: room.area_name,
    date: params.date,
    day_of_week: params.day_of_week,
    start_time: params.start_time,
    end_time: params.end_time,
    course_name: params.course_name,
    teacher: params.teacher,
    note: params.note,
    is_override: Boolean(params.room_id && params.room_id !== schedule.room_id),
    conflicts: [],
  }
}

/**
 * 把「课表 + 周模板 + 单日覆盖」展开成 [from, to] 内每天的具体课块。
 * - 日期先夹到课表的生效区间；
 * - 当天有 cancel：不排；有 replace：只用覆盖时段；
 * - 否则按 `day_of_week` 取周模板；
 * - `slot.room_id` 非空表示调课到其它房间。
 */
export function expandBoardBlocks(payload: BoardPayload, from: string, to: string): BoardBlock[] {
  const context = buildContext(payload)
  const slotsByScheduleDay = new Map<string, BoardWeeklySlot[]>()
  for (const slot of payload.weekly_slots) {
    const key = `${slot.schedule_id}|${slot.day_of_week}`
    const list = slotsByScheduleDay.get(key) ?? []
    list.push(slot)
    slotsByScheduleDay.set(key, list)
  }

  const blocks: BoardBlock[] = []
  for (const schedule of payload.schedules) {
    if (!schedule.is_active) continue
    const start = schedule.start_date > from ? schedule.start_date : from
    const end = schedule.end_date < to ? schedule.end_date : to
    if (start > end) continue
    for (let date = start; date <= end; date = addDays(date, 1)) {
      const dayOfWeek = isoWeekday(date)
      const exception = context.exceptionsByKey.get(`${schedule.id}|${date}`)
      if (exception) {
        if (exception.kind === 'cancel') continue
        for (const slot of exception.slots) {
          const block = makeBlock(context, {
            source: 'exception',
            schedule_id: schedule.id,
            slot_id: slot.id,
            room_id: slot.room_id,
            date,
            day_of_week: dayOfWeek,
            start_time: slot.start_time,
            end_time: slot.end_time,
            course_name: slot.course_name,
            teacher: slot.teacher,
            note: slot.note,
          })
          if (block) blocks.push(block)
        }
        continue
      }
      for (const slot of slotsByScheduleDay.get(`${schedule.id}|${dayOfWeek}`) ?? []) {
        const block = makeBlock(context, {
          source: 'template',
          schedule_id: schedule.id,
          slot_id: slot.id,
          room_id: slot.room_id,
          date,
          day_of_week: dayOfWeek,
          start_time: slot.start_time,
          end_time: slot.end_time,
          course_name: slot.course_name,
          teacher: slot.teacher,
          note: slot.note,
        })
        if (block) blocks.push(block)
      }
    }
  }

  blocks.sort(
    (left, right) =>
      left.date.localeCompare(right.date) ||
      left.start_time.localeCompare(right.start_time) ||
      left.room_code.localeCompare(right.room_code),
  )
  return blocks
}

function overlaps(left: BoardBlock, right: BoardBlock): boolean {
  return left.start_time < right.end_time && right.start_time < left.end_time
}

/**
 * 计算冲突并写回每个课块：
 * - 同一天同一房间时段重叠 → error（房间冲突）；
 * - 同一天同一教师（同名）时段重叠且不在同一房间 → warning（教师冲突）。
 */
export function attachBoardConflicts(blocks: BoardBlock[]): BoardConflict[] {
  for (const block of blocks) block.conflicts = []
  const conflicts: BoardConflict[] = []

  const byRoom = new Map<string, BoardBlock[]>()
  const byTeacher = new Map<string, BoardBlock[]>()
  for (const block of blocks) {
    const roomKey = `${block.date}|${block.room_id}`
    const roomList = byRoom.get(roomKey) ?? []
    roomList.push(block)
    byRoom.set(roomKey, roomList)

    const teacher = block.teacher.trim()
    if (teacher) {
      const teacherKey = `${block.date}|${teacher}`
      const teacherList = byTeacher.get(teacherKey) ?? []
      teacherList.push(block)
      byTeacher.set(teacherKey, teacherList)
    }
  }

  for (const list of byRoom.values()) {
    for (let index = 0; index < list.length; index += 1) {
      for (let next = index + 1; next < list.length; next += 1) {
        const left = list[index]!
        const right = list[next]!
        if (!overlaps(left, right)) continue
        const conflict: BoardConflict = {
          kind: 'room',
          severity: 'error',
          date: left.date,
          message: `${left.date} ${left.room_code} 房间时段重叠：${left.course_name}（${left.start_time}–${left.end_time}）与 ${right.course_name}（${right.start_time}–${right.end_time}）`,
          block_keys: [left.key, right.key],
        }
        left.conflicts.push(conflict)
        right.conflicts.push(conflict)
        conflicts.push(conflict)
      }
    }
  }

  for (const list of byTeacher.values()) {
    for (let index = 0; index < list.length; index += 1) {
      for (let next = index + 1; next < list.length; next += 1) {
        const left = list[index]!
        const right = list[next]!
        if (left.room_id === right.room_id) continue
        if (!overlaps(left, right)) continue
        const conflict: BoardConflict = {
          kind: 'teacher',
          severity: 'warning',
          date: left.date,
          message: `${left.date} 教师「${left.teacher.trim()}」同一时段被安排在 ${left.room_code} 与 ${right.room_code}（${left.start_time}–${left.end_time}）`,
          block_keys: [left.key, right.key],
        }
        left.conflicts.push(conflict)
        right.conflicts.push(conflict)
        conflicts.push(conflict)
      }
    }
  }

  return conflicts
}

export function blocksOfDate(blocks: BoardBlock[], date: string): BoardBlock[] {
  return blocks.filter((block) => block.date === date)
}

export function blocksOfWeek(blocks: BoardBlock[], weekStart: string): BoardBlock[] {
  const dates = new Set(weekDates(weekStart))
  return blocks.filter((block) => dates.has(block.date))
}

export function sortRooms(rooms: BoardRoom[]): BoardRoom[] {
  return [...rooms].sort(
    (left, right) =>
      left.area_name.localeCompare(right.area_name, 'zh-CN') ||
      left.code.localeCompare(right.code, 'zh-CN'),
  )
}

export function groupRoomsByArea(rooms: BoardRoom[]): Array<{ area: string; rooms: BoardRoom[] }> {
  const groups = new Map<string, BoardRoom[]>()
  for (const room of sortRooms(rooms)) {
    const key = `${room.area_id}|${room.area_name}`
    const list = groups.get(key) ?? []
    list.push(room)
    groups.set(key, list)
  }
  return [...groups.entries()].map(([key, list]) => ({
    area: key.split('|')[1] ?? '',
    rooms: list,
  }))
}
