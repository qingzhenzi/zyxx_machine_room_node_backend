/**
 * 排课计划（路径 2 Phase A）手写类型与纯函数。
 *
 * 契约：docs/design/schedule-plan-implementation-contract.md §2/§3。
 * 这里不导入 `@/api/schema`（自动生成文件），避免 openapi 重生成时相互耦合；
 * 房间下拉直接使用 `@/api/schema` 的 `Room` 类型（页面传入）。
 */
import type { Room } from '@/api/schema'
import { weekdayLabel } from '@/utils/format'

export type PlanStatus = 'draft' | 'published'
export type MasterDataType = 'course' | 'teacher' | 'class'

export interface PageMeta {
  page?: number
  page_size?: number
  total?: number
}

/* ------------------------------- 学期 ------------------------------- */

export interface Term {
  id: string
  code: string
  name: string
  start_date: string
  end_date: string
  is_active: boolean
  usage?: number
  created_at?: string
  updated_at?: string
}

export interface TermPage extends PageMeta {
  items?: Term[]
}

export interface TermInput {
  code: string
  name: string
  start_date: string
  end_date: string
  is_active?: boolean
}

export interface TermListParams extends PageMeta {
  keyword?: string
  include_inactive?: boolean
}

/* ------------------------------ 主数据 ------------------------------ */

export interface MasterMeta {
  created_at?: string
  updated_at?: string
  usage?: number
  metadata?: Record<string, unknown> | null
}

export interface Course extends MasterMeta {
  id: string
  code: string
  name: string
  color: string
  is_active: boolean
}

export interface Teacher extends MasterMeta {
  id: string
  code: string
  name: string
  phone: string
  email: string
  is_active: boolean
}

export interface SchoolClass extends MasterMeta {
  id: string
  code: string
  name: string
  grade: string
  is_active: boolean
}

export type MasterRecord = Course | Teacher | SchoolClass

export interface MasterPage<T> extends PageMeta {
  items?: T[]
}

export interface MasterListParams extends PageMeta {
  keyword?: string
  include_inactive?: boolean
}

export interface CourseInput {
  code: string
  name: string
  color?: string
  is_active?: boolean
}

export interface TeacherInput {
  code: string
  name: string
  phone?: string
  email?: string
  is_active?: boolean
}

export interface ClassInput {
  code: string
  name: string
  grade?: string
  is_active?: boolean
}

/** 主数据新建/编辑弹窗的通用表单值（按类型只用其中一部分字段）。 */
export interface MasterFormValue {
  code: string
  name: string
  is_active: boolean
  color: string
  phone: string
  email: string
  grade: string
}

/* ------------------------------ 排课计划 ---------------------------- */

export interface PlanView {
  id: string
  term_id: string
  term_name: string
  name: string
  status: PlanStatus
  version: number
  published_at: string | null
  published_by: string | null
  created_by: string | null
  entry_count: number
  course_count: number
  room_count: number
  created_at: string
  updated_at: string
}

export interface PlanPage extends PageMeta {
  items?: PlanView[]
}

export interface PlanEntry {
  id: string
  plan_id: string
  room_id: string
  room_code: string
  room_name: string
  area_id: string
  area_name: string
  day_of_week: number
  start_time: string
  end_time: string
  course_id: string
  course_code: string
  course_name: string
  teacher_id: string | null
  teacher_name: string
  class_id: string | null
  class_name: string
  note: string
}

export interface NewPlanEntry {
  room_id: string
  day_of_week: number
  start_time: string
  end_time: string
  course_id: string
  teacher_id?: string | null
  class_id?: string | null
  note?: string
}

export interface PlanDetail {
  plan: PlanView
  entries: PlanEntry[]
}

export interface CreatePlanRequest {
  term_id: string
  name: string
}

export interface UpdatePlanRequest {
  name: string
}

export interface PlanListParams extends PageMeta {
  term_id?: string
  status?: PlanStatus
}

export interface PublishResult {
  plan: PlanView
  schedules_rebuilt: number
  slots_written: number
  occurrences: number
}

/* --------------------------- 计划条目编辑器 -------------------------- */

/** 本地草稿行：只用于页面编辑，保存时转换成 NewPlanEntry。 */
export interface PlanEntryDraft {
  key: string
  /** 服务端条目 id；本地新增为 null。 */
  id: string | null
  room_id: string
  day_of_week: number
  start_time: string
  end_time: string
  course_id: string
  teacher_id: string | null
  class_id: string | null
  note: string
  /* 展示用快照（本地新增时按主数据实时解析）。 */
  room_code: string
  room_name: string
  area_name: string
  course_name: string
  teacher_name: string
  class_name: string
}

export interface PlanEntryFormValue {
  room_id: string
  day_of_week: number
  start_time: string
  end_time: string
  course_id: string
  teacher_id: string
  class_id: string
  note: string
}

/* ------------------------------ 常量/元数据 ------------------------- */

export const MASTER_TYPES: Array<{ value: MasterDataType; label: string }> = [
  { value: 'course', label: '课程' },
  { value: 'teacher', label: '教师' },
  { value: 'class', label: '班级' },
]

export function masterTypeLabel(type: MasterDataType): string {
  return MASTER_TYPES.find((item) => item.value === type)?.label ?? '主数据'
}

export function planStatusMeta(status: PlanStatus): {
  label: string
  tone: 'amber' | 'emerald'
} {
  return status === 'published'
    ? { label: '已发布', tone: 'emerald' }
    : { label: '草稿', tone: 'amber' }
}

export function masterRecordIsActive(record: MasterRecord): boolean {
  return record.is_active
}

/* ------------------------------ 工具函数 ---------------------------- */

const TIME_PATTERN = /^([01]\d|2[0-3]):[0-5]\d$/

export function isTimeString(value: string): boolean {
  return TIME_PATTERN.test(value)
}

export function minutesOf(time: string): number {
  const [hour, minute] = time.split(':').map(Number)
  return (hour ?? 0) * 60 + (minute ?? 0)
}

export function formatMinutes(total: number): string {
  const clamped = ((Math.round(total) % 1440) + 1440) % 1440
  return `${String(Math.floor(clamped / 60)).padStart(2, '0')}:${String(clamped % 60).padStart(2, '0')}`
}

let draftSequence = 0

export function nextDraftKey(): string {
  draftSequence += 1
  return `draft-${Date.now()}-${draftSequence}`
}

export function emptyEntryForm(): PlanEntryFormValue {
  return {
    room_id: '',
    day_of_week: 1,
    start_time: '08:00',
    end_time: '09:40',
    course_id: '',
    teacher_id: '',
    class_id: '',
    note: '',
  }
}

export function formOfDraft(draft: PlanEntryDraft): PlanEntryFormValue {
  return {
    room_id: draft.room_id,
    day_of_week: draft.day_of_week,
    start_time: draft.start_time,
    end_time: draft.end_time,
    course_id: draft.course_id,
    teacher_id: draft.teacher_id ?? '',
    class_id: draft.class_id ?? '',
    note: draft.note,
  }
}

export function draftToNewEntry(draft: PlanEntryDraft): NewPlanEntry {
  return {
    room_id: draft.room_id,
    day_of_week: draft.day_of_week,
    start_time: draft.start_time,
    end_time: draft.end_time,
    course_id: draft.course_id,
    teacher_id: draft.teacher_id || null,
    class_id: draft.class_id || null,
    note: draft.note.trim(),
  }
}

function resolveRoom(rooms: Room[], roomId: string): Room | null {
  return rooms.find((room) => room.id === roomId) ?? null
}

function resolveMasterName(records: MasterRecord[], id: string | null, fallback = ''): string {
  if (!id) return ''
  const record = records.find((item) => item.id === id)
  return record ? record.name : fallback
}

export function draftFromEntry(entry: PlanEntry): PlanEntryDraft {
  return {
    key: entry.id || nextDraftKey(),
    id: entry.id || null,
    room_id: entry.room_id,
    day_of_week: entry.day_of_week,
    start_time: entry.start_time.slice(0, 5),
    end_time: entry.end_time.slice(0, 5),
    course_id: entry.course_id,
    teacher_id: entry.teacher_id,
    class_id: entry.class_id,
    note: entry.note ?? '',
    room_code: entry.room_code,
    room_name: entry.room_name,
    area_name: entry.area_name,
    course_name: entry.course_name,
    teacher_name: entry.teacher_name ?? '',
    class_name: entry.class_name ?? '',
  }
}

export function draftFromForm(
  value: PlanEntryFormValue,
  rooms: Room[],
  records: MasterRecord[],
): PlanEntryDraft {
  const room = resolveRoom(rooms, value.room_id)
  return {
    key: nextDraftKey(),
    id: null,
    room_id: value.room_id,
    day_of_week: value.day_of_week,
    start_time: value.start_time,
    end_time: value.end_time,
    course_id: value.course_id,
    teacher_id: value.teacher_id || null,
    class_id: value.class_id || null,
    note: value.note.trim(),
    room_code: room?.code ?? '—',
    room_name: room?.name ?? '未知房间',
    area_name: room?.area_name ?? '—',
    course_name: resolveMasterName(records, value.course_id, '未知课程'),
    teacher_name: resolveMasterName(records, value.teacher_id || null),
    class_name: resolveMasterName(records, value.class_id || null),
  }
}

/** 页面统一排序：房间码 → 星期 → 开始时间。 */
export function sortDrafts(drafts: PlanEntryDraft[]): PlanEntryDraft[] {
  return [...drafts].sort(
    (left, right) =>
      left.room_code.localeCompare(right.room_code, 'zh-CN') ||
      left.day_of_week - right.day_of_week ||
      left.start_time.localeCompare(right.start_time) ||
      left.end_time.localeCompare(right.end_time),
  )
}

/** 草稿指纹（与主键/展示字段无关），用于判断是否有未保存修改。 */
export function entriesSignature(drafts: PlanEntryDraft[]): string {
  const normalized = drafts
    .map((draft) => {
      const entry = draftToNewEntry(draft)
      return [
        entry.room_id,
        entry.day_of_week,
        entry.start_time,
        entry.end_time,
        entry.course_id,
        entry.teacher_id ?? '',
        entry.class_id ?? '',
        entry.note ?? '',
      ].join('|')
    })
    .sort()
  return normalized.join('\n')
}

function overlaps(
  left: { room_id: string; day_of_week: number; start_time: string; end_time: string },
  right: { room_id: string; day_of_week: number; start_time: string; end_time: string },
): boolean {
  if (left.room_id !== right.room_id || left.day_of_week !== right.day_of_week) return false
  return left.start_time < right.end_time && right.start_time < left.end_time
}

/** 同一房间同一天不允许时段重叠（后端也会校验，这里提前拦截）。 */
export function validateEntryOverlap(
  drafts: PlanEntryDraft[],
  value: PlanEntryFormValue,
  ignoreKey?: string,
): string {
  const target = {
    room_id: value.room_id,
    day_of_week: value.day_of_week,
    start_time: value.start_time,
    end_time: value.end_time,
  }
  for (const draft of drafts) {
    if (ignoreKey && draft.key === ignoreKey) continue
    if (!overlaps(target, draft)) continue
    return `与 ${draft.room_code} ${weekdayLabel(draft.day_of_week)} ${draft.start_time}–${draft.end_time} 的条目时段重叠`
  }
  return ''
}

export function validateEntryForm(value: PlanEntryFormValue): string {
  if (!value.room_id) return '请选择房间'
  if (value.day_of_week < 1 || value.day_of_week > 7) return '星期取值需在 1–7 之间'
  if (!isTimeString(value.start_time) || !isTimeString(value.end_time)) return '时间格式需为 HH:MM'
  if (minutesOf(value.start_time) >= minutesOf(value.end_time)) return '开始时间必须早于结束时间'
  if (!value.course_id) return '请选择课程'
  return ''
}

export function draftSummary(drafts: PlanEntryDraft[]): {
  entries: number
  rooms: number
  courses: number
  minutes: number
} {
  const rooms = new Set<string>()
  const courses = new Set<string>()
  let minutes = 0
  for (const draft of drafts) {
    rooms.add(draft.room_id)
    courses.add(draft.course_id)
    minutes += Math.max(0, minutesOf(draft.end_time) - minutesOf(draft.start_time))
  }
  return { entries: drafts.length, rooms: rooms.size, courses: courses.size, minutes }
}

export function termContainsToday(term: Term, today: string): boolean {
  return term.is_active && term.start_date <= today && term.end_date >= today
}

/** 默认选中年份包含今天的 active 学期；否则取第一个 active，再否则第一条。 */
export function pickDefaultTerm(terms: Term[], today: string): Term | null {
  return (
    terms.find((term) => termContainsToday(term, today)) ??
    terms.find((term) => term.is_active) ??
    terms[0] ??
    null
  )
}

/* --------------------------- 主数据表单辅助 ------------------------- */

export function emptyMasterForm(type: MasterDataType): MasterFormValue {
  return {
    code: '',
    name: '',
    is_active: true,
    color: type === 'course' ? '#3a5bd9' : '',
    phone: '',
    email: '',
    grade: '',
  }
}

export function masterFormOf(type: MasterDataType, record: MasterRecord): MasterFormValue {
  const base = emptyMasterForm(type)
  base.code = record.code
  base.name = record.name
  base.is_active = record.is_active
  if (type === 'course') base.color = (record as Course).color ?? ''
  if (type === 'teacher') {
    base.phone = (record as Teacher).phone ?? ''
    base.email = (record as Teacher).email ?? ''
  }
  if (type === 'class') base.grade = (record as SchoolClass).grade ?? ''
  return base
}

export function validateMasterForm(type: MasterDataType, value: MasterFormValue): string {
  if (!value.code.trim()) return '请填写编码'
  if (!value.name.trim()) return '请填写名称'
  if (type === 'teacher' && value.email.trim() && !value.email.includes('@')) {
    return '邮箱格式不正确'
  }
  return ''
}

/* ------------------------- 计划详情响应归一化 ----------------------- */

/**
 * `GET /schedule-plans/{id}` 契约描述为「计划 + 条目」。
 * 兼容两种后端返回：`{plan, entries}` 与扁平 `PlanView & {entries}`。
 */
export function normalizePlanDetail(raw: unknown): PlanDetail {
  const body = (raw ?? {}) as Partial<PlanView> & { plan?: PlanView; entries?: PlanEntry[] }
  const entries = Array.isArray(body.entries) ? body.entries : []
  if (body.plan && typeof body.plan === 'object') {
    return { plan: body.plan, entries }
  }
  return { plan: body as unknown as PlanView, entries }
}
