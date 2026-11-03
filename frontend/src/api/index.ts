import axios, { type AxiosResponse, type InternalAxiosRequestConfig } from 'axios'

import type {
  PlanDetail,
  PlanEntry,
  PlanListParams,
  PlanPage,
  PlanView,
  PublishResult,
  Term,
  TermInput,
  TermListParams,
  TermPage,
  Course,
  CourseInput,
  MasterListParams,
  MasterPage,
  Teacher,
  TeacherInput,
  SchoolClass,
  ClassInput,
  NewPlanEntry,
  CreatePlanRequest,
  UpdatePlanRequest,
} from '@/features/schedulePlans/types'
import { normalizePlanDetail } from '@/features/schedulePlans/types'
import type {
  BoardPayload,
  BoardQuery,
  EnsureSchedulesReply,
  EnsureSchedulesRequest,
} from '@/features/scheduleBoard/boardTypes'

import type {
  Announcement,
  AttachmentPresignReply,
  AttachmentPresignRequest,
  AttachmentRegisterRequest,
  AnnouncementInput,
  Area,
  AssignImageRequest,
  AssignTicketRequest,
  AuditLogPage,
  CollectionPage,
  CommentTicketRequest,
  CreateAreaRequest,
  CreateImageRequest,
  CreateInspectionPlanRequest,
  CreateRoleRequest,
  CreateRoomRequest,
  CreateScheduleRequest,
  CreateTicketRequest,
  CreateTicketRuleRequest,
  CreateUserRequest,
  DeviceInfo,
  DevicePage,
  DeviceTokenReply,
  DeviceTokenRequest,
  EffectiveAgentPolicyReply,
  MachineDetail,
  MachinePage,
  MachineSnapshotPoint,
  MachineTokenRequest,
  EventRule,
  ImageArtifact,
  ImageAssignment,
  ImageDetail,
  ImagePage,
  InspectionPlanView,
  InspectionRun,
  ManifestReply,
  NewEventRule,
  NewWeeklySlot,
  Occurrence,
  Permission,
  PresenceDeliveryList,
  PresenceEvent,
  PresenceEventList,
  PresignObjectReply,
  PublishImageRequest,
  RebuildResult,
  ReplayResult,
  RoleSummary,
  Room,
  ScopeAreaOption,
  RoomDevice,
  RoomMachine,
  RoomPage,
  Schedule,
  ScheduleEventDetail,
  ScheduleEventPage,
  ScheduleException,
  SchedulePage,
  SetPermissionsRequest,
  SetRolesRequest,
  StorageObject,
  StorageObjectReply,
  StorageObjectRequest,
  SubmitManifestReply,
  SubmitManifestRequest,
  Ticket,
  TicketAssignee,
  TicketAttachment,
  TicketDetail,
  TicketEvent,
  TicketPage,
  TicketRule,
  TicketRuleList,
  TicketScopeUser,
  TicketStats,
  TicketStreamEvent,
  TicketSummary,
  TokenResponse,
  Transfer,
  TransferPage,
  TransitionTicketRequest,
  UpdateAgentConfigReply,
  UpdateAgentConfigRequest,
  UpdateAreaRequest,
  UpdateInspectionPlanRequest,
  UpdateRoomRequest,
  UpdateScheduleRequest,
  UpdateTicketRequest,
  UpdateTicketRuleRequest,
  UpdateTicketScopesRequest,
  UpdateUserRequest,
  UpsertExceptionRequest,
  UserPage,
  UserProfile,
  UserSession,
  UserSummary,
  VerifyReply,
  WeeklySlot,
} from './schema'
import {
  clearSession,
  getAccessToken,
  getRefreshToken,
  isRemembered,
  setAccessToken,
  setRefreshToken,
} from './session'
import type {
  AnnouncementListParams,
  AreaListParams,
  AuditListParams,
  AuthSession,
  CollectionListParams,
  DeviceListParams,
  HealthStatus,
  ImageListParams,
  MachineDetailParams,
  MachineListParams,
  OccurrenceParams,
  PresenceDeliveryParams,
  PresenceEventParams,
  ReadyStatus,
  RoomListParams,
  ScheduleEventListParams,
  ScheduleListParams,
  SearchResults,
  TicketEventStreamOptions,
  TicketListParams,
  TicketStatsParams,
  TransferListParams,
  UserListParams,
} from './types'

/** 默认走同源 `/api/v1`；部署到独立域名时用 VITE_API_BASE_URL 覆盖。 */
const baseURL = import.meta.env.VITE_API_BASE_URL || '/api/v1'

const http = axios.create({ baseURL, timeout: 20_000 })

/** 不带拦截器的实例，用于刷新令牌，避免 401 处理递归。 */
const bare = axios.create({ baseURL, timeout: 20_000 })

/** 探针接口在 `/api/v1` 之外，单独使用根路径实例。 */
function probeOrigin(): string {
  const configured = import.meta.env.VITE_API_BASE_URL
  if (!configured) return ''
  try {
    return new URL(configured, window.location.href).origin
  } catch {
    return ''
  }
}

const probe = axios.create({ baseURL: probeOrigin(), timeout: 5_000 })

/** 带 HTTP 状态码与后端错误码的异常，页面可以据此区分权限/不存在等场景。 */
export class ApiError extends Error {
  readonly status?: number
  readonly code?: string

  constructor(message: string, status?: number, code?: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
    this.code = code
  }
}

export function isForbidden(error: unknown): boolean {
  return error instanceof ApiError && error.status === 403
}

export function isNotFound(error: unknown): boolean {
  return error instanceof ApiError && error.status === 404
}

interface RetriableConfig extends InternalAxiosRequestConfig {
  __retried?: boolean
}

let unauthorizedHandler: (() => void) | null = null

/** 会话彻底失效（刷新令牌也没了）时通知路由层跳登录页。 */
export function setUnauthorizedHandler(handler: (() => void) | null) {
  unauthorizedHandler = handler
}

http.interceptors.request.use((config) => {
  const token = getAccessToken()
  if (token) config.headers.Authorization = `Bearer ${token}`
  return config
})

let refreshing: Promise<boolean> | null = null

/**
 * 用 refresh token 换一对新令牌。并发 401 共用同一个请求，
 * 避免多个刷新请求同时消费一次性刷新令牌而互相失效。
 */
async function refreshAccessToken(): Promise<boolean> {
  const refreshToken = getRefreshToken()
  if (!refreshToken) return false

  refreshing ??= (async () => {
    try {
      const { data } = await bare.post<TokenResponse>('/auth/refresh', {
        refresh_token: refreshToken,
      })
      setAccessToken(data.access_token)
      setRefreshToken(data.refresh_token, isRemembered())
      return true
    } catch {
      clearSession()
      return false
    } finally {
      refreshing = null
    }
  })()

  return refreshing
}

http.interceptors.response.use(
  (response) => response,
  async (error: unknown) => {
    if (!axios.isAxiosError(error)) throw error

    const config = error.config as RetriableConfig | undefined
    const status = error.response?.status
    const isAuthRequest = (config?.url ?? '').startsWith('/auth/')

    if (status === 401 && config && !config.__retried && !isAuthRequest) {
      config.__retried = true
      if (await refreshAccessToken()) {
        return http.request(config)
      }
      unauthorizedHandler?.()
    }

    throw error
  },
)

/** 后端错误是 `{ error: { code, message } }`，mock/代理错误可能是 `{ message }`。 */
function toApiError(error: unknown, fallback: string): ApiError {
  if (error instanceof ApiError) return error
  if (!axios.isAxiosError(error)) {
    return new ApiError(error instanceof Error && error.message ? error.message : fallback)
  }

  const payload = error.response?.data as
    | { message?: string; error?: { code?: string; message?: string } }
    | undefined
  const message = payload?.error?.message ?? payload?.message ?? error.message ?? fallback
  return new ApiError(message || fallback, error.response?.status, payload?.error?.code)
}

async function unwrap<T>(request: Promise<AxiosResponse<T>>): Promise<T> {
  try {
    const response = await request
    return response.data
  } catch (error) {
    throw toApiError(error, '请求失败')
  }
}

function toSession(raw: TokenResponse): AuthSession {
  return {
    accessToken: raw.access_token,
    refreshToken: raw.refresh_token,
    expiresIn: raw.expires_in,
    user: raw.user,
  }
}

export const api = {
  // ---- 认证 / 会话 ----
  login: (input: { username: string; password: string }) =>
    unwrap<TokenResponse>(http.post('/auth/login', input)).then(toSession),

  refresh: (refreshToken: string) =>
    unwrap<TokenResponse>(bare.post('/auth/refresh', { refresh_token: refreshToken })).then(
      toSession,
    ),

  logout: (refreshToken: string) =>
    unwrap<void>(http.post('/auth/logout', { refresh_token: refreshToken })),

  me: () => unwrap<UserProfile>(http.get('/auth/me')),

  forgotPassword: (email: string) => unwrap<void>(http.post('/auth/password/forgot', { email })),

  resetPassword: (input: { token: string; new_password: string }) =>
    unwrap<void>(http.post('/auth/password/reset', input)),

  requestEmailVerification: () => unwrap<void>(http.post('/auth/email/verification')),

  verifyEmail: (token: string) => unwrap<void>(http.post('/auth/email/verify', { token })),

  listSessions: () => unwrap<UserSession[]>(http.get('/auth/sessions')),

  revokeSession: (id: string) =>
    unwrap<void>(http.delete(`/auth/sessions/${encodeURIComponent(id)}`)),

  revokeAllSessions: () => unwrap<void>(http.delete('/auth/sessions')),

  // ---- 探针 ----
  health: () => unwrap<HealthStatus>(probe.get('/healthz')),

  ready: () => unwrap<ReadyStatus>(probe.get('/readyz')),

  // ---- 区域 ----
  listAreas: (params: AreaListParams = {}) =>
    unwrap<AreaPageWire>(http.get('/admin/areas', { params })),

  getArea: (id: string) => unwrap<Area>(http.get(`/admin/areas/${encodeURIComponent(id)}`)),

  createArea: (input: CreateAreaRequest) => unwrap<Area>(http.post('/admin/areas', input)),

  updateArea: (id: string, input: UpdateAreaRequest) =>
    unwrap<Area>(http.patch(`/admin/areas/${encodeURIComponent(id)}`, input)),

  deleteArea: (id: string) => unwrap<void>(http.delete(`/admin/areas/${encodeURIComponent(id)}`)),

  // ---- 房间 ----
  listRooms: (params: RoomListParams = {}) =>
    unwrap<RoomPage>(http.get('/admin/rooms', { params })),

  getRoom: (id: string) => unwrap<Room>(http.get(`/admin/rooms/${encodeURIComponent(id)}`)),

  createRoom: (input: CreateRoomRequest) => unwrap<Room>(http.post('/admin/rooms', input)),

  updateRoom: (id: string, input: UpdateRoomRequest) =>
    unwrap<Room>(http.patch(`/admin/rooms/${encodeURIComponent(id)}`, input)),

  deleteRoom: (id: string) => unwrap<void>(http.delete(`/admin/rooms/${encodeURIComponent(id)}`)),

  roomDevice: (id: string) =>
    unwrap<RoomDevice | null>(http.get(`/admin/rooms/${encodeURIComponent(id)}/device`)),

  bindRoomDevice: (id: string, deviceSlot: string | null) =>
    unwrap<Room>(
      http.put(`/admin/rooms/${encodeURIComponent(id)}/device`, { device_slot: deviceSlot }),
    ),

  issueDeviceToken: (id: string, input: DeviceTokenRequest = {}) =>
    unwrap<DeviceTokenReply>(
      http.post(`/admin/rooms/${encodeURIComponent(id)}/device-token`, input),
    ),

  // ---- 房间子机（P1：一个房间多台机器）----
  /** 房间机器列表：主设备在前，其余为子机。 */
  listRoomMachines: (id: string) =>
    unwrap<RoomMachine[]>(http.get(`/admin/rooms/${encodeURIComponent(id)}/devices`)),

  /** 新增子机并签发一次性注册 token（槽位由后端分配为 `{room_code}-m{n}`）。 */
  issueRoomMachineToken: (id: string, input: MachineTokenRequest = {}) =>
    unwrap<DeviceTokenReply>(
      http.post(`/admin/rooms/${encodeURIComponent(id)}/devices/token`, input),
    ),

  /** 解除房间与子机的绑定（只删关系，不吊销证书）。 */
  removeRoomMachine: (id: string, slot: string) =>
    unwrap<void>(
      http.delete(`/admin/rooms/${encodeURIComponent(id)}/devices/${encodeURIComponent(slot)}`),
    ),

  // ---- 设备 ----
  listDevices: (params: DeviceListParams = {}) =>
    unwrap<DevicePage>(http.get('/admin/devices', { params })),

  getDevice: (serial: string) =>
    unwrap<DeviceInfo>(http.get(`/admin/devices/${encodeURIComponent(serial)}`)),

  // ---- 课表 ----
  listSchedules: (params: ScheduleListParams = {}) =>
    unwrap<SchedulePage>(http.get('/admin/schedules', { params })),

  /** 排课工作台聚合读模型：一次拿到区域/房间/课表/周模板/单日覆盖。 */
  scheduleBoard: (params: BoardQuery) =>
    unwrap<BoardPayload>(http.get('/admin/schedule-board', { params })),

  /** 幂等为房间补默认课表（工作台在无课表房间新建课程前调用）。 */
  ensureSchedules: (input: EnsureSchedulesRequest) =>
    unwrap<EnsureSchedulesReply>(http.post('/admin/schedules/ensure', input)),

  getSchedule: (id: string) =>
    unwrap<Schedule>(http.get(`/admin/schedules/${encodeURIComponent(id)}`)),

  createSchedule: (input: CreateScheduleRequest) =>
    unwrap<Schedule>(http.post('/admin/schedules', input)),

  updateSchedule: (id: string, input: UpdateScheduleRequest) =>
    unwrap<Schedule>(http.patch(`/admin/schedules/${encodeURIComponent(id)}`, input)),

  deleteSchedule: (id: string) =>
    unwrap<void>(http.delete(`/admin/schedules/${encodeURIComponent(id)}`)),

  weeklySlots: (id: string) =>
    unwrap<WeeklySlot[]>(http.get(`/admin/schedules/${encodeURIComponent(id)}/weekly-slots`)),

  replaceWeeklySlots: (id: string, slots: NewWeeklySlot[]) =>
    unwrap<WeeklySlot[]>(
      http.put(`/admin/schedules/${encodeURIComponent(id)}/weekly-slots`, { slots }),
    ),

  exceptions: (id: string, params: OccurrenceParams = {}) =>
    unwrap<ScheduleException[]>(
      http.get(`/admin/schedules/${encodeURIComponent(id)}/exceptions`, { params }),
    ),

  upsertException: (id: string, date: string, input: UpsertExceptionRequest) =>
    unwrap<ScheduleException>(
      http.put(`/admin/schedules/${encodeURIComponent(id)}/exceptions/${date}`, input),
    ),

  deleteException: (id: string, date: string) =>
    unwrap<void>(http.delete(`/admin/schedules/${encodeURIComponent(id)}/exceptions/${date}`)),

  occurrences: (id: string, params: OccurrenceParams = {}) =>
    unwrap<Occurrence[]>(
      http.get(`/admin/schedules/${encodeURIComponent(id)}/occurrences`, { params }),
    ),

  roomOccurrences: (roomId: string, params: OccurrenceParams = {}) =>
    unwrap<Occurrence[]>(
      http.get(`/admin/rooms/${encodeURIComponent(roomId)}/occurrences`, { params }),
    ),

  rebuildSchedule: (id: string) =>
    unwrap<RebuildResult>(http.post(`/admin/schedules/${encodeURIComponent(id)}/rebuild`)),

  // ---- 排课计划（Phase A：学期 / 主数据 / 计划 / 发布）----
  listTerms: (params: TermListParams = {}) =>
    unwrap<TermPage>(http.get('/admin/terms', { params })),

  createTerm: (input: TermInput) => unwrap<Term>(http.post('/admin/terms', input)),

  updateTerm: (id: string, input: Partial<TermInput>) =>
    unwrap<Term>(http.patch(`/admin/terms/${encodeURIComponent(id)}`, input)),

  deleteTerm: (id: string) => unwrap<void>(http.delete(`/admin/terms/${encodeURIComponent(id)}`)),

  listTeachers: (params: MasterListParams = {}) =>
    unwrap<MasterPage<Teacher>>(http.get('/admin/teachers', { params })),

  createTeacher: (input: TeacherInput) => unwrap<Teacher>(http.post('/admin/teachers', input)),

  updateTeacher: (id: string, input: Partial<TeacherInput>) =>
    unwrap<Teacher>(http.patch(`/admin/teachers/${encodeURIComponent(id)}`, input)),

  deleteTeacher: (id: string) =>
    unwrap<void>(http.delete(`/admin/teachers/${encodeURIComponent(id)}`)),

  listClasses: (params: MasterListParams = {}) =>
    unwrap<MasterPage<SchoolClass>>(http.get('/admin/classes', { params })),

  createClass: (input: ClassInput) => unwrap<SchoolClass>(http.post('/admin/classes', input)),

  updateClass: (id: string, input: Partial<ClassInput>) =>
    unwrap<SchoolClass>(http.patch(`/admin/classes/${encodeURIComponent(id)}`, input)),

  deleteClass: (id: string) =>
    unwrap<void>(http.delete(`/admin/classes/${encodeURIComponent(id)}`)),

  listCourses: (params: MasterListParams = {}) =>
    unwrap<MasterPage<Course>>(http.get('/admin/courses', { params })),

  createCourse: (input: CourseInput) => unwrap<Course>(http.post('/admin/courses', input)),

  updateCourse: (id: string, input: Partial<CourseInput>) =>
    unwrap<Course>(http.patch(`/admin/courses/${encodeURIComponent(id)}`, input)),

  deleteCourse: (id: string) =>
    unwrap<void>(http.delete(`/admin/courses/${encodeURIComponent(id)}`)),

  listSchedulePlans: (params: PlanListParams = {}) =>
    unwrap<PlanPage>(http.get('/admin/schedule-plans', { params })),

  /** 计划详情（计划 + 条目）；兼容 `{plan,entries}` 与扁平返回。 */
  getSchedulePlan: async (id: string): Promise<PlanDetail> =>
    normalizePlanDetail(
      await unwrap<unknown>(http.get(`/admin/schedule-plans/${encodeURIComponent(id)}`)),
    ),

  createSchedulePlan: (input: CreatePlanRequest) =>
    unwrap<PlanView>(http.post('/admin/schedule-plans', input)),

  updateSchedulePlan: (id: string, input: UpdatePlanRequest) =>
    unwrap<PlanView>(http.patch(`/admin/schedule-plans/${encodeURIComponent(id)}`, input)),

  deleteSchedulePlan: (id: string) =>
    unwrap<void>(http.delete(`/admin/schedule-plans/${encodeURIComponent(id)}`)),

  cloneSchedulePlan: (id: string) =>
    unwrap<PlanView>(http.post(`/admin/schedule-plans/${encodeURIComponent(id)}/clone`)),

  schedulePlanEntries: (id: string) =>
    unwrap<PlanEntry[]>(http.get(`/admin/schedule-plans/${encodeURIComponent(id)}/entries`)),

  replaceSchedulePlanEntries: (id: string, entries: NewPlanEntry[]) =>
    unwrap<PlanEntry[]>(
      http.put(`/admin/schedule-plans/${encodeURIComponent(id)}/entries`, { entries }),
    ),

  publishSchedulePlan: (id: string) =>
    unwrap<PublishResult>(http.post(`/admin/schedule-plans/${encodeURIComponent(id)}/publish`)),

  // ---- 课表事件 / 触发规则 ----
  listScheduleEvents: (params: ScheduleEventListParams = {}) =>
    unwrap<ScheduleEventPage>(http.get('/admin/schedule-events', { params })),

  getScheduleEvent: (id: string) =>
    unwrap<ScheduleEventDetail>(http.get(`/admin/schedule-events/${encodeURIComponent(id)}`)),

  replayScheduleEvent: (id: string) =>
    unwrap<ReplayResult>(http.post(`/admin/schedule-events/${encodeURIComponent(id)}/replay`)),

  retryScheduleDelivery: (eventId: string, deliveryId: number) =>
    unwrap<void>(
      http.post(
        `/admin/schedule-events/${encodeURIComponent(eventId)}/deliveries/${deliveryId}/retry`,
      ),
    ),

  roomEventRules: (roomId: string) =>
    unwrap<EventRule[]>(http.get(`/admin/rooms/${encodeURIComponent(roomId)}/event-rules`)),

  replaceRoomEventRules: (roomId: string, rules: NewEventRule[]) =>
    unwrap<EventRule[]>(
      http.put(`/admin/rooms/${encodeURIComponent(roomId)}/event-rules`, { rules }),
    ),

  // ---- 机器总览（room-agent 上报）----
  listMachines: (params: MachineListParams = {}) =>
    unwrap<MachinePage>(http.get('/admin/machines', { params })),

  getMachine: (serial: string, params: MachineDetailParams = {}) =>
    unwrap<MachineDetail>(http.get(`/admin/machines/${encodeURIComponent(serial)}`, { params })),

  machineSnapshots: (serial: string, limit = 50) =>
    unwrap<MachineSnapshotPoint[]>(
      http.get(`/admin/machines/${encodeURIComponent(serial)}/snapshots`, {
        params: { limit },
      }),
    ),

  // ---- 设备在线事件 ----
  listDeviceEvents: (params: PresenceEventParams = {}) =>
    unwrap<PresenceEventList>(http.get('/admin/device-events', { params })),

  listPresenceDeliveries: (params: PresenceDeliveryParams = {}) =>
    unwrap<PresenceDeliveryList>(http.get('/admin/device-presence-deliveries', { params })),

  // ---- 镜像 ----
  listImages: (params: ImageListParams = {}) =>
    unwrap<ImagePage>(http.get('/admin/images', { params })),

  getImage: (id: string) =>
    unwrap<ImageDetail>(http.get(`/admin/images/${encodeURIComponent(id)}`)),

  createImage: (input: CreateImageRequest) =>
    unwrap<ImageArtifact>(http.post('/admin/images', input)),

  getManifest: (id: string) =>
    unwrap<ManifestReply>(http.get(`/admin/images/${encodeURIComponent(id)}/manifest`)),

  submitManifest: (id: string, input: SubmitManifestRequest) =>
    unwrap<SubmitManifestReply>(
      http.post(`/admin/images/${encodeURIComponent(id)}/manifest`, input),
    ),

  verifyImage: (id: string) =>
    unwrap<VerifyReply>(http.post(`/admin/images/${encodeURIComponent(id)}/verify`)),

  publishImage: (id: string, input: PublishImageRequest) =>
    unwrap<ImageArtifact>(http.post(`/admin/images/${encodeURIComponent(id)}/publish`, input)),

  assignImage: (roomId: string, imageId: string, input: AssignImageRequest = {}) =>
    unwrap<ImageAssignment>(
      http.put(
        `/admin/rooms/${encodeURIComponent(roomId)}/images/${encodeURIComponent(imageId)}`,
        input,
      ),
    ),

  unassignImage: (roomId: string, imageId: string) =>
    unwrap<void>(
      http.delete(
        `/admin/rooms/${encodeURIComponent(roomId)}/images/${encodeURIComponent(imageId)}`,
      ),
    ),

  // ---- 存储 ----
  presignObject: (input: StorageObjectRequest) =>
    unwrap<PresignObjectReply>(http.post('/admin/storage/objects/presign', input)),

  registerObject: (input: StorageObjectRequest) =>
    unwrap<StorageObject>(http.post('/admin/storage/objects', input)),

  getStorageObject: (sha256: string) =>
    unwrap<StorageObjectReply>(http.get(`/admin/storage/objects/${encodeURIComponent(sha256)}`)),

  listTransfers: (params: TransferListParams = {}) =>
    unwrap<TransferPage>(http.get('/admin/transfers', { params })),

  retryTransfer: (id: string) =>
    unwrap<Transfer>(http.post(`/admin/transfers/${encodeURIComponent(id)}/retry`)),

  listCollections: (params: CollectionListParams = {}) =>
    unwrap<CollectionPage>(http.get('/admin/collections', { params })),

  // ---- 用户 / 角色 / 权限 / 审计 ----
  listUsers: (params: UserListParams = {}) =>
    unwrap<UserPage>(http.get('/admin/users', { params })),

  createUser: (input: CreateUserRequest) => unwrap<UserSummary>(http.post('/admin/users', input)),

  updateUser: (id: string, input: UpdateUserRequest) =>
    unwrap<UserSummary>(http.patch(`/admin/users/${encodeURIComponent(id)}`, input)),

  setUserRoles: (id: string, roleCodes: string[]) =>
    unwrap<UserSummary>(
      http.put(`/admin/users/${encodeURIComponent(id)}/roles`, {
        role_codes: roleCodes,
      } satisfies SetRolesRequest),
    ),

  listRoles: () => unwrap<RoleSummary[]>(http.get('/admin/roles')),

  createRole: (input: CreateRoleRequest) => unwrap<RoleSummary>(http.post('/admin/roles', input)),

  deleteRole: (id: string) => unwrap<void>(http.delete(`/admin/roles/${encodeURIComponent(id)}`)),

  setRolePermissions: (id: string, permissionCodes: string[]) =>
    unwrap<RoleSummary>(
      http.put(`/admin/roles/${encodeURIComponent(id)}/permissions`, {
        permission_codes: permissionCodes,
      } satisfies SetPermissionsRequest),
    ),

  listPermissions: () => unwrap<Permission[]>(http.get('/admin/permissions')),

  listAuditLogs: (params: AuditListParams = {}) =>
    unwrap<AuditLogPage>(http.get('/admin/audit-logs', { params })),

  // ---- 公告 ----
  listAdminAnnouncements: (params: AnnouncementListParams = {}) =>
    unwrap<AnnouncementPageWire>(http.get('/admin/announcements', { params })),

  createAnnouncement: (input: AnnouncementInput) =>
    unwrap<Announcement>(http.post('/admin/announcements', input)),

  updateAnnouncement: (id: string, input: AnnouncementInput) =>
    unwrap<Announcement>(http.patch(`/admin/announcements/${encodeURIComponent(id)}`, input)),

  deleteAnnouncement: (id: string) =>
    unwrap<void>(http.delete(`/admin/announcements/${encodeURIComponent(id)}`)),

  // ---- 节点配置 ----
  listAgentConfigs: () =>
    unwrap<{ items: AgentConfigWire[]; total: number }>(http.get('/admin/agent-configs')),

  updateAgentConfig: (input: UpdateAgentConfigRequest) =>
    unwrap<UpdateAgentConfigReply>(http.put('/admin/agent-configs', input)),

  effectiveAgentConfig: (roomId: string) =>
    unwrap<EffectiveAgentPolicyReply>(
      http.get('/admin/agent-configs/effective', { params: { room_id: roomId } }),
    ),

  // ---- 工单 ----
  listTickets: (params: TicketListParams = {}) =>
    unwrap<TicketPage>(http.get('/admin/tickets', { params })),

  ticketSummary: () => unwrap<TicketSummary>(http.get('/admin/tickets/summary')),

  ticketStats: (params: TicketStatsParams = {}) =>
    unwrap<TicketStats>(http.get('/admin/tickets/stats', { params })),

  ticketAssignees: () => unwrap<TicketAssignee[]>(http.get('/admin/tickets/assignees')),

  getTicket: (id: string) =>
    unwrap<TicketDetail>(http.get(`/admin/tickets/${encodeURIComponent(id)}`)),

  createTicket: (input: CreateTicketRequest) => unwrap<Ticket>(http.post('/admin/tickets', input)),

  updateTicket: (id: string, input: UpdateTicketRequest) =>
    unwrap<Ticket>(http.patch(`/admin/tickets/${encodeURIComponent(id)}`, input)),

  claimTicket: (id: string) =>
    unwrap<Ticket>(http.post(`/admin/tickets/${encodeURIComponent(id)}/claim`)),

  assignTicket: (id: string, input: AssignTicketRequest) =>
    unwrap<Ticket>(http.post(`/admin/tickets/${encodeURIComponent(id)}/assign`, input)),

  transitionTicket: (id: string, input: TransitionTicketRequest) =>
    unwrap<Ticket>(http.post(`/admin/tickets/${encodeURIComponent(id)}/transition`, input)),

  commentTicket: (id: string, input: CommentTicketRequest) =>
    unwrap<TicketEvent>(http.post(`/admin/tickets/${encodeURIComponent(id)}/comments`, input)),

  /** 导出 CSV（同列表筛选；返回 Blob，由页面生成下载）。 */
  exportTickets: async (params: TicketListParams = {}) => {
    const response = await http.get('/admin/tickets/export', { params, responseType: 'blob' })
    return response.data as Blob
  },

  // ---- 工单自动规则 ----
  listTicketRules: () => unwrap<TicketRuleList>(http.get('/admin/ticket-rules')),

  createTicketRule: (input: CreateTicketRuleRequest) =>
    unwrap<TicketRule>(http.post('/admin/ticket-rules', input)),

  updateTicketRule: (id: string, input: UpdateTicketRuleRequest) =>
    unwrap<TicketRule>(http.patch(`/admin/ticket-rules/${encodeURIComponent(id)}`, input)),

  deleteTicketRule: (id: string) =>
    unwrap<void>(http.delete(`/admin/ticket-rules/${encodeURIComponent(id)}`)),

  // ---- 工单数据范围（P1 Wave2）----
  listTicketScopes: () => unwrap<TicketScopeUser[]>(http.get('/admin/ticket-scopes')),

  setTicketScopes: (userId: string, input: UpdateTicketScopesRequest) =>
    unwrap<TicketScopeUser>(http.put(`/admin/ticket-scopes/${encodeURIComponent(userId)}`, input)),

  // ---- 工单范围区域选项（不要求 area:read）----
  listTicketScopeAreas: () => unwrap<ScopeAreaOption[]>(http.get('/admin/ticket-scopes/areas')),

  // ---- 工单附件（P1 Wave2）----
  listTicketAttachments: (ticketId: string) =>
    unwrap<TicketAttachment[]>(
      http.get(`/admin/tickets/${encodeURIComponent(ticketId)}/attachments`),
    ),

  /** 后端代理上传（同源，避免对象存储 CORS；≤20MiB）。 */
  uploadTicketAttachment: (ticketId: string, file: File) => {
    const contentType = file.type || 'application/octet-stream'
    return unwrap<TicketAttachment>(
      http.post(`/admin/tickets/${encodeURIComponent(ticketId)}/attachments/upload`, file, {
        params: { filename: file.name, content_type: contentType },
        headers: { 'Content-Type': contentType },
        timeout: 120_000,
      }),
    )
  },

  presignTicketAttachment: (ticketId: string, input: AttachmentPresignRequest) =>
    unwrap<AttachmentPresignReply>(
      http.post(`/admin/tickets/${encodeURIComponent(ticketId)}/attachments/presign`, input),
    ),

  registerTicketAttachment: (ticketId: string, input: AttachmentRegisterRequest) =>
    unwrap<TicketAttachment>(
      http.post(`/admin/tickets/${encodeURIComponent(ticketId)}/attachments`, input),
    ),

  /** 后端代理下载附件（同源、Content-Disposition: attachment）。 */
  downloadTicketAttachment: (ticketId: string, attachmentId: string) =>
    unwrap<Blob>(
      http.get(
        `/admin/tickets/${encodeURIComponent(ticketId)}/attachments/${encodeURIComponent(attachmentId)}/download`,
        { responseType: 'blob' },
      ),
    ),

  deleteTicketAttachment: (ticketId: string, attachmentId: string) =>
    unwrap<void>(
      http.delete(
        `/admin/tickets/${encodeURIComponent(ticketId)}/attachments/${encodeURIComponent(attachmentId)}`,
      ),
    ),

  // ---- 巡检计划（P1 Wave3）----
  listInspectionPlans: () => unwrap<InspectionPlanView[]>(http.get('/admin/inspection-plans')),

  createInspectionPlan: (input: CreateInspectionPlanRequest) =>
    unwrap<InspectionPlanView>(http.post('/admin/inspection-plans', input)),

  updateInspectionPlan: (id: string, input: UpdateInspectionPlanRequest) =>
    unwrap<InspectionPlanView>(
      http.patch(`/admin/inspection-plans/${encodeURIComponent(id)}`, input),
    ),

  deleteInspectionPlan: (id: string) =>
    unwrap<void>(http.delete(`/admin/inspection-plans/${encodeURIComponent(id)}`)),

  runInspectionPlan: (id: string) =>
    unwrap<InspectionRun>(http.post(`/admin/inspection-plans/${encodeURIComponent(id)}/run`)),

  listInspectionRuns: (params?: { plan_id?: string; limit?: number }) =>
    unwrap<InspectionRun[]>(http.get('/admin/inspection-runs', { params })),

  // ---- 全局搜索（命令面板）----
  search: async (keyword: string): Promise<SearchResults> => {
    const trimmed = keyword.trim()
    if (!trimmed) {
      return { rooms: [], devices: [], images: [], schedules: [], announcements: [] }
    }

    const [rooms, devices, images, schedules, announcements] = await Promise.all([
      http.get<RoomPage>('/admin/rooms', { params: { keyword: trimmed, page_size: 5 } }),
      http.get<DevicePage>('/admin/devices', { params: { keyword: trimmed, page_size: 5 } }),
      http.get<ImagePage>('/admin/images', { params: { name: trimmed, page_size: 5 } }),
      http.get<SchedulePage>('/admin/schedules', { params: { page_size: 5 } }),
      http.get<AnnouncementPageWire>('/admin/announcements', {
        params: { keyword: trimmed, page_size: 5 },
      }),
    ])

    return {
      rooms: rooms.data.items ?? [],
      devices: devices.data.items ?? [],
      images: images.data.items ?? [],
      schedules: (schedules.data.items ?? []).filter((item) =>
        `${item.name} ${item.room_name}`.toLowerCase().includes(trimmed.toLowerCase()),
      ),
      announcements: announcements.data.items ?? [],
    }
  },
}

/** 兼容 OpenAPI 里没有单独命名的 Page/返回结构。 */
interface AreaPageWire {
  items?: Area[]
  page?: number
  page_size?: number
  total?: number
}

interface AnnouncementPageWire {
  items?: Announcement[]
  page?: number
  page_size?: number
  total?: number
}

interface AgentConfigWire {
  id: string
  scope_type: 'global' | 'area' | 'room'
  scope_id: string | null
  policy: Record<string, unknown>
  revision: number
  updated_by: string | null
  created_at: string
  updated_at: string
}

/** 设备在线事件 SSE（浏览器 EventSource 不能带 Authorization，因此用 fetch 流式读取）。 */
export interface DeviceEventStreamOptions {
  afterId?: number | null
  signal: AbortSignal
  onEvent: (event: PresenceEvent) => void
  onHeartbeat?: () => void
}

export async function streamDeviceEvents(options: DeviceEventStreamOptions): Promise<void> {
  const params = new URLSearchParams()
  if (options.afterId != null) params.set('after_id', String(options.afterId))
  const query = params.size ? `?${params.toString()}` : ''
  const token = getAccessToken()

  const response = await fetch(`${baseURL}/admin/device-events/stream${query}`, {
    headers: token ? { Authorization: `Bearer ${token}` } : {},
    signal: options.signal,
  })

  if (!response.ok || !response.body) {
    throw new ApiError('设备事件流连接失败', response.status)
  }

  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })

    let boundary = buffer.indexOf('\n\n')
    while (boundary >= 0) {
      const chunk = buffer.slice(0, boundary)
      buffer = buffer.slice(boundary + 2)
      boundary = buffer.indexOf('\n\n')
      parseSseChunk(chunk, options)
    }
  }
}

function parseSseChunk(chunk: string, options: DeviceEventStreamOptions) {
  let eventName = 'message'
  const dataLines: string[] = []
  for (const line of chunk.split('\n')) {
    if (line.startsWith('event:')) eventName = line.slice(6).trim()
    else if (line.startsWith('data:')) dataLines.push(line.slice(5).trimStart())
  }
  if (!dataLines.length) return

  if (eventName === 'heartbeat') {
    options.onHeartbeat?.()
    return
  }
  if (eventName !== 'device.presence') return

  try {
    options.onEvent(JSON.parse(dataLines.join('\n')) as PresenceEvent)
  } catch {
    // 忽略无法解析的帧，后续帧继续处理
  }
}

/** 机器快照 SSE 流（列表/详情页实时刷新；浏览器 EventSource 不能带 Authorization，用 fetch 流式读取）。 */
export interface MachineSnapshotStreamOptions {
  /** 从该快照 id 之后补发；缺省 = 只接收连接之后的新快照。 */
  afterId?: number | null
  /** 只订阅某台机器。 */
  serial?: string
  signal: AbortSignal
  /** HTTP 响应建立、SSE 连接就绪时回调（此时还没有数据帧）。 */
  onOpen?: () => void
  onSnapshot: (snapshot: MachineSnapshotPoint) => void
  onHeartbeat?: () => void
}

export async function streamMachineSnapshots(options: MachineSnapshotStreamOptions): Promise<void> {
  const params = new URLSearchParams()
  if (options.afterId != null) params.set('after_id', String(options.afterId))
  if (options.serial) params.set('serial', options.serial)
  const query = params.size ? `?${params.toString()}` : ''
  const token = getAccessToken()

  const response = await fetch(`${baseURL}/admin/machines/stream${query}`, {
    headers: {
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      Accept: 'text/event-stream',
    },
    signal: options.signal,
  })

  if (!response.ok || !response.body) {
    throw new ApiError('机器实时流连接失败', response.status)
  }
  options.onOpen?.()

  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })

    let boundary = buffer.indexOf('\n\n')
    while (boundary >= 0) {
      const chunk = buffer.slice(0, boundary)
      buffer = buffer.slice(boundary + 2)
      boundary = buffer.indexOf('\n\n')
      parseMachineSseChunk(chunk, options)
    }
  }
}

function parseMachineSseChunk(chunk: string, options: MachineSnapshotStreamOptions) {
  let eventName = 'message'
  const dataLines: string[] = []
  for (const line of chunk.split('\n')) {
    if (line.startsWith('event:')) eventName = line.slice(6).trim()
    else if (line.startsWith('data:')) dataLines.push(line.slice(5).trimStart())
  }
  if (!dataLines.length) return
  if (eventName === 'ready' || eventName === 'heartbeat') {
    options.onOpen?.()
    options.onHeartbeat?.()
    return
  }
  if (eventName !== 'machine.snapshot') return
  try {
    options.onSnapshot(JSON.parse(dataLines.join('\n')) as MachineSnapshotPoint)
  } catch {
    // 忽略无法解析的帧
  }
}

/** 工单事件 SSE 流（列表/详情实时刷新；浏览器 EventSource 不能带 Authorization）。 */
export async function streamTicketEvents(options: TicketEventStreamOptions): Promise<void> {
  const params = new URLSearchParams()
  if (options.afterEventId != null) params.set('after_event_id', String(options.afterEventId))
  const query = params.size ? `?${params.toString()}` : ''
  const token = getAccessToken()

  const response = await fetch(`${baseURL}/admin/tickets/stream${query}`, {
    headers: {
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      Accept: 'text/event-stream',
    },
    signal: options.signal,
  })

  if (!response.ok || !response.body) {
    throw new ApiError('工单事件流连接失败', response.status)
  }

  options.onOpen?.()
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''

  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })

    let boundary = buffer.indexOf('\n\n')
    while (boundary >= 0) {
      const chunk = buffer.slice(0, boundary)
      buffer = buffer.slice(boundary + 2)
      boundary = buffer.indexOf('\n\n')
      parseTicketSseChunk(chunk, options)
    }
  }
}

function parseTicketSseChunk(chunk: string, options: TicketEventStreamOptions) {
  let eventName = 'message'
  const dataLines: string[] = []
  for (const line of chunk.split('\n')) {
    if (line.startsWith('event:')) eventName = line.slice(6).trim()
    else if (line.startsWith('data:')) dataLines.push(line.slice(5).trimStart())
  }
  if (eventName === 'heartbeat' || eventName === 'ready') {
    options.onOpen?.()
    options.onHeartbeat?.()
    return
  }
  if (!dataLines.length || eventName !== 'ticket.event') return
  try {
    options.onEvent(JSON.parse(dataLines.join('\n')) as TicketStreamEvent)
  } catch {
    // 忽略无法解析的帧
  }
}
