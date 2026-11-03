import type {
  Announcement,
  ArtifactStatus,
  DeviceInfo,
  DeviceStatus,
  ImageArtifact,
  Room,
  Schedule,
  TicketCategory,
  TicketPriority,
  TicketSource,
  TicketStatus,
  TicketStreamEvent,
  UserProfile,
} from './schema'

/** 登录后前端持有的会话；字段与后端 TokenResponse 对齐。 */
export interface AuthSession {
  accessToken: string
  refreshToken: string
  expiresIn: number
  user: UserProfile
}

/** 当前登录用户（后端 UserProfile 原样保留 snake_case）。 */
export type AuthUser = UserProfile

export interface PageParams {
  page?: number
  page_size?: number
}

export interface AreaListParams extends PageParams {
  keyword?: string
  include_inactive?: boolean
}

export interface RoomListParams extends PageParams {
  area_id?: string
  keyword?: string
  include_inactive?: boolean
}

export interface DeviceListParams extends PageParams {
  area_id?: string
  room_id?: string
  status?: DeviceStatus
  assigned?: boolean
  keyword?: string
}

export interface ScheduleListParams extends PageParams {
  room_id?: string
}

export interface ScheduleEventListParams extends PageParams {
  room_id?: string
  kind?: string
  from?: string
  to?: string
}

export interface ImageListParams extends PageParams {
  kind?: string
  status?: ArtifactStatus
  name?: string
}

export interface UserListParams extends PageParams {
  keyword?: string
}

export interface AuditListParams extends PageParams {
  action?: string
  resource_type?: string
}

export interface AnnouncementListParams extends PageParams {
  status?: 'draft' | 'published' | 'archived'
  keyword?: string
}

export interface TransferListParams extends PageParams {
  room_id?: string
}

export interface CollectionListParams extends PageParams {
  room_id?: string
}

export interface OccurrenceParams {
  from?: string
  to?: string
}

export interface PresenceEventParams {
  limit?: number
  after_id?: number
}

export interface MachineListParams extends PageParams {
  area_id?: string
  room_id?: string
  keyword?: string
  /** ok / warning / critical / missing（从未上报）/ unknown */
  health?: string
}

export interface MachineDetailParams {
  history_limit?: number
}

export interface PresenceDeliveryParams {
  status?: 'pending' | 'delivered' | 'failed'
  limit?: number
}

/** 工单列表筛选参数；`mine`/`active`/`overdue` 是后端快捷条件。 */
export interface TicketListParams extends PageParams {
  status?: TicketStatus
  priority?: TicketPriority
  category?: TicketCategory
  source?: TicketSource
  assignee_id?: string
  unassigned?: boolean
  mine?: boolean
  active?: boolean
  overdue?: boolean
  area_id?: string
  room_id?: string
  serial?: string
  keyword?: string
  from?: string
  to?: string
}

/** 工单历史分析时间范围（默认近 30 天）。 */
export interface TicketStatsParams {
  from?: string
  to?: string
}

/** 工单 SSE（fetch 流式读取；浏览器 EventSource 不能带 Authorization）。 */
export interface TicketEventStreamOptions {
  /** 从该 ticket_events.id 之后补发；缺省 = 只接收连接之后的新事件。 */
  afterEventId?: number | null
  signal: AbortSignal
  onEvent: (event: TicketStreamEvent) => void
  onOpen?: () => void
  onHeartbeat?: () => void
}

/** 服务存活探针：/healthz。 */
export interface HealthStatus {
  status: string
  instance: string
}

/** 依赖就绪探针：/readyz。 */
export interface ReadyStatus {
  status: string
  instance: string
  database: boolean
  cache: boolean
}

/** 概览与全局搜索用的聚合结果。 */
export interface SearchResults {
  rooms: Room[]
  devices: DeviceInfo[]
  images: ImageArtifact[]
  schedules: Schedule[]
  announcements: Announcement[]
}

export interface KpiSummary {
  areas: number
  rooms: number
  devices: number
  onlineDevices: number
  schedules: number
  images: number
  publishedImages: number
  announcements: number
}
