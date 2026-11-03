// 本文件由 docs/openapi.json 生成，勿手改。
// 重新生成：python3 docs/openapi-types.py

export type AgentConfigScopeType = 'global' | 'area' | 'room'

export type ArtifactStatus =
  | 'uploaded'
  | 'manifest_ready'
  | 'verifying'
  | 'verified'
  | 'published'
  | 'deprecated'
  | 'revoked'

export type CollectionStatus = 'pending' | 'uploaded' | 'verified' | 'rejected'

export type Compression = 'none' | 'zstd' | 'gzip'

export type DeviceOnlineState = 'online' | 'offline'

export type DeviceStatus = 'pending' | 'issued' | 'active' | 'rejected' | 'revoked'

export type ImageChannel = 'dev' | 'test' | 'prod'

export type ImageFormat = 'docker_archive' | 'oci_layout' | 'tar' | 'qcow2' | 'raw'

export type ImageKind = 'docker' | 'oci' | 'lxc_rootfs' | 'vm_disk'

export type InspectionRunStatus = 'running' | 'ok' | 'issues' | 'failed'

export type InspectionScopeType = 'global' | 'area' | 'room'

export type PlanStatus = 'draft' | 'published'

export type TicketCategory =
  | 'machine_health'
  | 'device_offline'
  | 'delivery_failed'
  | 'distribution_failed'
  | 'security'
  | 'other'

export type TicketEventKind =
  | 'created'
  | 'claimed'
  | 'assigned'
  | 'status_changed'
  | 'priority_changed'
  | 'commented'
  | 'occurred'
  | 'updated'
  | 'reminded'

export type TicketPriority = 'urgent' | 'high' | 'normal' | 'low'

export type TicketSource = 'auto' | 'manual'

export type TicketStatus = 'open' | 'in_progress' | 'resolved' | 'closed' | 'cancelled'

export type TransferStatus = 'pending' | 'running' | 'completed' | 'failed' | 'expired'

export interface AgentConfig {
  created_at: string
  id: string
  policy: AgentPolicyPatch
  revision: number
  scope_id: string | null
  scope_type: AgentConfigScopeType
  updated_at: string
  updated_by: string | null
}

export interface AgentConfigList {
  items: AgentConfig[]
  total: number
}

export interface AgentPolicy {
  cache_max_bytes: number
  download_concurrency: number
  lxc_auto_start: boolean
  poll_interval_secs: number
}

export interface AgentPolicyPatch {
  cache_max_bytes?: number
  download_concurrency?: number
  lxc_auto_start?: boolean
  poll_interval_secs?: number
}

export interface Announcement {
  author_id?: string | null
  author_name?: string | null
  content: string
  created_at: string
  id: string
  is_pinned: boolean
  published_at?: string | null
  status: 'draft' | 'published' | 'archived'
  title: string
  updated_at: string
}

export interface AnnouncementInput {
  content: string
  is_pinned?: boolean
  status?: 'draft' | 'published' | 'archived'
  title: string
}

export interface AnnouncementPage {
  items?: Announcement[]
  page?: number
  page_size?: number
  total?: number
}

export interface Area {
  code: string
  created_at: string
  description: string
  id: string
  is_active: boolean
  metadata: Record<string, unknown>
  name: string
  parent_id?: string | null
  timezone: string
  updated_at: string
  webhook_url?: string | null
}

export interface AreaPage {
  items?: Area[]
  page?: number
  page_size?: number
  total?: number
}

export interface AssignImageRequest {
  channel?: ImageChannel
}

export interface AssignTicketRequest {
  assignee_id: string | null
}

export interface AttachmentPresignReply {
  expires_at: string
  object_key: string
  upload_url: string
}

export interface AttachmentPresignRequest {
  content_type: string
  filename: string
  sha256: string
  size: number
}

export interface AttachmentRegisterRequest {
  content_type: string
  filename: string
  object_key: string
  sha256: string
  size: number
}

export interface AuditLog {
  action?: string
  actor_display_name?: string
  actor_id?: string | null
  actor_username?: string
  created_at?: string
  detail?: Record<string, unknown>
  id?: string
  ip?: string | null
  resource_id?: string | null
  resource_type?: string
}

export interface AuditLogPage {
  items?: AuditLog[]
  page?: number
  page_size?: number
  total?: number
}

export interface BoardArea {
  code: string
  id: string
  name: string
  timezone: string
}

export interface BoardDevice {
  online: boolean | null
  serial: string
  status: 'pending' | 'issued' | 'active' | 'rejected' | 'revoked' | null
}

export interface BoardException {
  exception_date: string
  id: string
  kind: 'cancel' | 'replace'
  note: string
  schedule_id: string
  slots: BoardExceptionSlot[]
}

export interface BoardExceptionSlot {
  course_name: string
  end_time: string
  id: string
  note: string
  room_id: string | null
  start_time: string
  teacher: string
}

export interface BoardRoom {
  area_id: string
  area_name: string
  capacity: number
  code: string
  device: BoardDevice | null
  device_slot: string | null
  id: string
  is_active: boolean
  name: string
  timezone: string
}

export interface BoardSchedule {
  end_date: string
  id: string
  is_active: boolean
  name: string
  room_id: string
  start_date: string
  timezone: string
}

export interface BoardWeeklySlot {
  course_name: string
  day_of_week: number
  end_time: string
  id: string
  note: string
  room_id: string | null
  schedule_id: string
  start_time: string
  teacher: string
}

export interface ClassPage {
  items?: ClassView[]
  page?: number
  page_size?: number
  total?: number
}

export interface ClassView {
  code: string
  created_at: string
  grade: string
  id: string
  is_active: boolean
  metadata: Record<string, unknown>
  name: string
  updated_at: string
  usage: number
}

export interface Collection {
  area_id?: string | null
  created_at?: string
  id?: string
  kind?: string
  object_key?: string
  room_id?: string | null
  sha256?: string
  size?: number
  status?: CollectionStatus
}

export interface CollectionPage {
  items?: Collection[]
  page?: number
  page_size?: number
  total?: number
}

export interface CommentTicketRequest {
  body: string
}

export interface CoursePage {
  items?: CourseView[]
  page?: number
  page_size?: number
  total?: number
}

export interface CourseView {
  code: string
  color: string
  created_at: string
  id: string
  is_active: boolean
  metadata: Record<string, unknown>
  name: string
  updated_at: string
  usage: number
}

export interface CreateAreaRequest {
  code: string
  description?: string
  is_active?: boolean
  metadata?: Record<string, unknown>
  name: string
  parent_id?: string | null
  timezone?: string
  webhook_url?: string | null
}

export interface CreateClassRequest {
  code: string
  grade?: string
  is_active?: boolean
  metadata?: Record<string, unknown>
  name: string
}

export interface CreateCourseRequest {
  code: string
  color?: string
  is_active?: boolean
  metadata?: Record<string, unknown>
  name: string
}

export interface CreateImageRequest {
  compression?: Compression
  format: ImageFormat
  kind: ImageKind
  name: string
  platform: string
  version: string
}

export interface CreateInspectionPlanRequest {
  enabled?: boolean
  interval_hours?: number
  name: string
  scope_id?: string | null
  scope_type?: InspectionScopeType
}

export interface CreateRoleRequest {
  code: string
  description?: string
  name: string
  permission_codes?: string[]
}

export interface CreateRoomRequest {
  area_id: string
  capacity?: number
  code: string
  device_slot?: string | null
  is_active?: boolean
  metadata?: Record<string, unknown>
  name: string
  webhook_url?: string | null
}

export interface CreateSchedulePlanRequest {
  name: string
  term_id: string
}

export interface CreateScheduleRequest {
  end_date: string
  is_active?: boolean
  name: string
  room_id: string
  start_date: string
}

export interface CreateTeacherRequest {
  code: string
  email?: string
  is_active?: boolean
  metadata?: Record<string, unknown>
  name: string
  phone?: string
}

export interface CreateTermRequest {
  code: string
  end_date: string
  is_active?: boolean
  name: string
  start_date: string
}

export interface CreateTicketRequest {
  area_id?: string | null
  category?: TicketCategory
  description?: string
  device_slot?: string | null
  priority?: TicketPriority
  room_id?: string | null
  serial?: string | null
  title: string
}

export interface CreateTicketRuleRequest {
  enabled?: boolean
  name: string
  priority?: TicketPriority
  scope_id?: string | null
  scope_type?: 'global' | 'area' | 'room'
  signal_type:
    | 'machine_health'
    | 'device_offline'
    | 'schedule_delivery'
    | 'presence_delivery'
    | 'transfer'
    | 'image_verify'
  thresholds?: Record<string, unknown>
}

export interface CreateUserRequest {
  display_name: string
  email?: string
  password: string
  role_codes?: string[]
  username: string
}

export interface DeviceInfo {
  activated_at: string | null
  area_id: string | null
  area_name: string | null
  client_info: Record<string, unknown> | null
  device_slot: string
  downloaded: boolean
  issued_at: string | null
  last_seen_at: string | null
  online: DeviceOnlineState | null
  revoked_at: string | null
  room_code: string | null
  room_id: string | null
  room_name: string | null
  serial: string
  status: DeviceStatus
}

export interface DevicePage {
  items?: DeviceInfo[]
  page?: number
  page_size?: number
  total?: number
}

export interface DeviceTokenReply {
  device_slot: string
  expires_at: string
  kind: 'auto' | 'approval'
  link: string
  token: string
}

export interface DeviceTokenRequest {
  kind?: 'auto' | 'approval'
  ttl_secs?: number
}

export interface EffectiveAgentPolicyReply {
  policy: AgentPolicy
  revision: number
  room_id: string
}

export interface EnsureSchedulesRequest {
  end_date: string
  name?: string
  room_ids: string[]
  start_date: string
}

export interface EnsureSchedulesResponse {
  created: Schedule[]
  existing: Schedule[]
}

export interface Error {
  error: { code: string; message: string }
}

export interface EventRule {
  at_time?: string | null
  config?: Record<string, unknown>
  created_at?: string
  enabled?: boolean
  id?: string
  offset_minutes?: number
  room_id?: string
  trigger_kind?:
    | 'occurrence_start'
    | 'occurrence_end'
    | 'occurrence_start_before'
    | 'daily_time'
    | 'room_occupied'
    | 'room_idle'
  updated_at?: string
}

export interface ImageArtifact {
  compression: Compression
  created_at: string
  created_by?: string | null
  format: ImageFormat
  id: string
  kind: ImageKind
  manifest_digest?: string | null
  name: string
  platform: string
  status: ArtifactStatus
  total_size: number
  updated_at: string
  version: string
}

export interface ImageAssignment {
  area_id?: string | null
  assigned_at?: string
  assigned_by?: string
  channel?: ImageChannel
  id?: string
  image_id?: string
  rollout_state?: string
  room_id?: string | null
  updated_at?: string
}

export interface ImageChannelRow {
  artifact_id?: string
  channel?: ImageChannel
  created_at?: string
  id?: string
  is_current?: boolean
}

export interface ImageChunk {
  index?: number
  manifest_digest?: string
  object_key?: string
  offset?: number
  sha256?: string
  size?: number
}

export interface ImageDetail extends ImageArtifact {
  channels?: ImageChannelRow[]
  chunks?: ImageChunk[]
  latest_verification?: ImageVerification | null
}

export interface ImagePage {
  items?: ImageArtifact[]
  page?: number
  page_size?: number
  total?: number
}

export interface ImageVerification {
  artifact_id?: string
  id?: string
  ok?: boolean
  report?: {
    checks?: { detail?: string; name?: string; ok?: boolean }[]
    errors?: string[]
    manifest_digest?: string
  }
  verified_at?: string
  verified_by?: string
}

export interface InspectionLastRun {
  issue_machines: number
  started_at: string
  status: InspectionRunStatus
  ticket_id: string | null
}

export interface InspectionPlan {
  created_at: string
  created_by: string | null
  created_by_name: string | null
  enabled: boolean
  id: string
  interval_hours: number
  last_run_at: string | null
  name: string
  next_run_at: string | null
  scope_id: string | null
  scope_type: InspectionScopeType
  updated_at: string
}

export interface InspectionPlanView extends InspectionPlan {
  last_run: InspectionLastRun | null
}

export interface InspectionRun {
  checked_machines: number
  error: string | null
  finished_at: string | null
  id: string
  issue_machines: number
  plan_id: string
  started_at: string
  status: InspectionRunStatus
  summary: Record<string, unknown>
  ticket_id: string | null
}

export interface LoginRequest {
  password: string
  username: string
}

export interface MachineDetail extends MachineSummary {
  history?: MachineSnapshotPoint[]
  latest?: MachineSnapshot | null
}

export interface MachineHealth {
  issues?: MachineIssue[]
  score?: number | null
  status: 'ok' | 'warning' | 'critical'
}

export interface MachineIssue {
  code: string
  detail?: string | null
  message: string
  severity: 'info' | 'warning' | 'critical'
}

export interface MachineMetrics {
  cpu_usage_percent?: number | null
  disk_usage_percent?: number | null
  memory_usage_percent?: number | null
  uptime_secs?: number | null
}

export interface MachinePage {
  items: MachineSummary[]
  page: number
  page_size: number
  total: number
}

export interface MachineReportReply {
  device_slot: string
  received_at: string
  serial: string
  snapshot_id: number
}

export interface MachineReportRequest {
  agent_version: string
  collected_at?: string
  device_slot: string
  health: MachineHealth
  hostname: string
  kernel_version?: string | null
  metrics?: MachineMetrics
  os_name?: string | null
  schema?: number
  sections?: Record<string, unknown>
}

export interface MachineReportSummary {
  agent_version: string
  collected_at: string
  cpu_usage_percent?: number | null
  disk_usage_percent?: number | null
  health_score?: number | null
  health_status: 'ok' | 'warning' | 'critical' | 'unknown'
  hostname: string
  kernel_version?: string | null
  memory_usage_percent?: number | null
  os_name?: string | null
  received_at: string
  uptime_secs?: number | null
}

export interface MachineSnapshot extends MachineSnapshotPoint {
  payload: Record<string, unknown>
}

export interface MachineSnapshotPoint {
  agent_version: string
  collected_at: string
  cpu_usage_percent?: number | null
  device_slot: string
  disk_usage_percent?: number | null
  health_score?: number | null
  health_status: 'ok' | 'warning' | 'critical'
  hostname: string
  id: number
  kernel_version?: string | null
  memory_usage_percent?: number | null
  os_name?: string | null
  received_at: string
  serial: string
  uptime_secs?: number | null
}

export interface MachineSummary {
  area_code: string | null
  area_id: string | null
  area_name: string | null
  device_slot: string
  last_seen_at: string | null
  online: 'online' | 'offline' | null
  report: MachineReportSummary | null
  report_stale: boolean
  room_code: string | null
  room_id: string | null
  room_name: string | null
  serial: string
  session_online: 'online' | 'offline' | null
  status: 'pending' | 'issued' | 'active' | 'rejected' | 'revoked'
}

export interface MachineTokenRequest {
  kind?: 'auto' | 'approval'
  label?: string
  ttl_secs?: number
}

export interface Manifest {
  annotations?: Record<string, unknown>
  chunks?: ManifestChunk[]
  compression?: Compression
  created_at?: string
  created_by?: string | null
  format?: ImageFormat
  kind?: ImageKind
  name?: string
  platform?: string
  runtime?: ManifestRuntime
  schema?: number
  total_size?: number
  version?: string
}

export interface ManifestChunk {
  index?: number
  object_key?: string
  offset?: number
  sha256?: string
  size?: number
}

export interface ManifestPart {
  index: number
  offset: number
  sha256: string
  size: number
}

export interface ManifestReply {
  artifact_id: string
  download_url: string
  expires_at: string
  manifest: Manifest
  manifest_digest: string
}

export interface ManifestRuntime {
  docker_image_ref?: string | null
  lxc_distro?: string | null
  lxc_release?: string | null
  vm_disk_size?: number | null
  vm_os?: string | null
}

export interface ManifestRuntimeInput {
  docker?: { image_ref?: string }
  lxc?: { distro?: string; release?: string }
  vm?: { disk_size?: number; os?: string }
}

export interface NewEventRule {
  at_time?: string | null
  config?: Record<string, unknown>
  enabled?: boolean
  offset_minutes?: number
  trigger_kind:
    | 'occurrence_start'
    | 'occurrence_end'
    | 'occurrence_start_before'
    | 'daily_time'
    | 'room_occupied'
    | 'room_idle'
}

export interface NewExceptionSlot {
  course_name: string
  end_time: string
  note?: string
  room_id?: string | null
  start_time: string
  teacher?: string
}

export interface NewWeeklySlot {
  course_name: string
  day_of_week: number
  end_time: string
  note?: string
  room_id?: string | null
  start_time: string
  teacher?: string
}

export interface Occurrence {
  course_name?: string
  end_at?: string
  end_local?: string
  id?: string
  note?: string
  occurrence_date?: string
  room_id?: string
  schedule_id?: string
  source?: 'weekly' | 'custom'
  source_slot_id?: string
  start_at?: string
  start_local?: string
  teacher?: string
  timezone?: string
}

export interface Permission {
  code?: string
  description?: string
  id?: string
  name?: string
}

export interface PlanDetailResponse {
  entries: PlanEntry[]
  plan: SchedulePlan
}

export interface PlanEntry {
  area_id: string
  area_name: string
  class_id: string | null
  class_name: string | null
  course_code: string
  course_id: string
  course_name: string
  day_of_week: number
  end_time: string
  id: string
  note: string
  plan_id: string
  room_code: string
  room_id: string
  room_name: string
  start_time: string
  teacher_id: string | null
  teacher_name: string | null
}

export interface PlanEntryInput {
  class_id?: string | null
  course_id: string
  day_of_week: number
  end_time: string
  note?: string
  room_id: string
  start_time: string
  teacher_id?: string | null
}

export interface PresenceDelivery {
  attempts: number
  event_id: number
  id: number
  last_attempt_at: string | null
  last_error: string | null
  next_attempt_at: string
  status: 'pending' | 'delivered' | 'failed'
  target_url: string
}

export interface PresenceDeliveryList {
  items: PresenceDelivery[]
  total: number
}

export interface PresenceEvent {
  area_id?: string | null
  current_state: 'online' | 'offline'
  device_slot: string
  id: number
  occurred_at: string
  payload: Record<string, unknown>
  previous_state: 'online' | 'offline'
  room_id?: string | null
  serial: string
}

export interface PresenceEventList {
  items: PresenceEvent[]
  total: number
}

export interface PresignObjectReply {
  bucket: string
  expires_at: string
  object_key: string
  upload_url: string
}

export interface PublishImageRequest {
  channel: ImageChannel
}

export interface PublishResultResponse {
  occurrences: number
  plan: SchedulePlan
  schedules_rebuilt: number
  slots_written: number
}

export interface RebuildResult {
  occurrences?: number
}

export interface RefreshRequest {
  refresh_token: string
}

export interface ReplaceEventRulesRequest {
  rules: NewEventRule[]
}

export interface ReplacePlanEntriesRequest {
  entries: PlanEntryInput[]
}

export interface ReplaceWeeklySlotsRequest {
  slots: NewWeeklySlot[]
}

export interface ReplayResult {
  reset?: number
}

export interface RoleSummary {
  code?: string
  description?: string
  id?: string
  is_system?: boolean
  name?: string
  permissions?: string[]
}

export interface Room {
  area_id: string
  area_name: string
  capacity: number
  code: string
  created_at: string
  device: RoomDevice | null
  device_slot?: string | null
  id: string
  is_active: boolean
  metadata: Record<string, unknown>
  name: string
  timezone: string
  updated_at: string
  webhook_url?: string | null
}

export interface RoomDevice {
  device_slot: string
  last_seen_at: string | null
  online: 'online' | 'offline' | null
  pending_token: boolean
  serial: string | null
  status: 'pending' | 'issued' | 'active' | 'rejected' | 'revoked' | null
}

export interface RoomMachine {
  created_at?: string | null
  device_slot: string
  is_primary: boolean
  label?: string | null
  last_seen_at: string | null
  online: 'online' | 'offline' | null
  pending_token: boolean
  serial: string | null
  status: 'pending' | 'issued' | 'active' | 'rejected' | 'revoked' | null
}

export interface RoomPage {
  items?: Room[]
  page?: number
  page_size?: number
  total?: number
}

export interface Schedule {
  created_at: string
  created_by?: string | null
  end_date: string
  id: string
  is_active: boolean
  name: string
  room_id: string
  room_name: string
  start_date: string
  timezone: string
  updated_at: string
}

export interface ScheduleBoard {
  areas: BoardArea[]
  exceptions: BoardException[]
  from: string
  rooms: BoardRoom[]
  schedules: BoardSchedule[]
  to: string
  weekly_slots: BoardWeeklySlot[]
}

export interface ScheduleEvent {
  created_at?: string
  event_key?: string
  event_kind?: string
  id?: string
  occurrence_id?: string | null
  payload?: Record<string, unknown>
  room_id?: string
  rule_id?: string | null
  schedule_id?: string | null
  timezone?: string
  title?: string
  triggered_at?: string
  triggered_at_local?: string
}

export interface ScheduleEventDelivery {
  attempts?: number
  created_at?: string
  delivered_at?: string | null
  event_id?: string
  id?: number
  last_error?: string | null
  next_attempt_at?: string
  status?: 'pending' | 'delivered' | 'failed' | 'skipped'
  target?: 'pubsub' | 'downlink' | 'webhook'
}

export interface ScheduleEventDetail {
  deliveries?: ScheduleEventDelivery[]
  event?: ScheduleEvent
}

export interface ScheduleEventPage {
  items?: ScheduleEvent[]
  page?: number
  page_size?: number
  total?: number
}

export interface ScheduleException {
  created_at?: string
  exception_date?: string
  id?: string
  kind?: 'cancel' | 'replace'
  note?: string
  schedule_id?: string
  slots?: ScheduleExceptionSlot[]
  updated_at?: string
}

export interface ScheduleExceptionSlot {
  course_name?: string
  end_time?: string
  exception_id?: string
  id?: string
  note?: string
  room_id?: string | null
  room_name?: string | null
  start_time?: string
  teacher?: string
}

export interface SchedulePage {
  items?: Schedule[]
  page?: number
  page_size?: number
  total?: number
}

export interface SchedulePlan {
  course_count: number
  created_at: string
  created_by: string | null
  entry_count: number
  id: string
  name: string
  published_at: string | null
  published_by: string | null
  room_count: number
  status: PlanStatus
  term_id: string
  term_name: string
  updated_at: string
  version: number
}

export interface SchedulePlanPage {
  items?: SchedulePlan[]
  page?: number
  page_size?: number
  total?: number
}

export interface ScopeAreaOption {
  code: string
  id: string
  name: string
}

export interface SetPermissionsRequest {
  permission_codes?: string[]
}

export interface SetRolesRequest {
  role_codes?: string[]
}

export interface StorageObject {
  bucket: string
  content_type: string
  created_at: string
  id: string
  object_key: string
  sha256: string
  size: number
}

export interface StorageObjectReply {
  download_url: string
  expires_at: string
  object: StorageObject
}

export interface StorageObjectRequest {
  content_type?: string | null
  sha256: string
  size: number
}

export interface SubmitManifestReply {
  artifact: ImageArtifact
  manifest: Manifest
  manifest_digest: string
}

export interface SubmitManifestRequest {
  annotations?: Record<string, unknown>
  parts: ManifestPart[]
  runtime?: ManifestRuntimeInput
  total_size: number
}

export interface TeacherPage {
  items?: TeacherView[]
  page?: number
  page_size?: number
  total?: number
}

export interface TeacherView {
  code: string
  created_at: string
  email: string
  id: string
  is_active: boolean
  metadata: Record<string, unknown>
  name: string
  phone: string
  updated_at: string
  usage: number
}

export interface TermPage {
  items?: TermView[]
  page?: number
  page_size?: number
  total?: number
}

export interface TermView {
  code: string
  created_at: string
  end_date: string
  id: string
  is_active: boolean
  name: string
  start_date: string
  updated_at: string
  usage: number
}

export interface Ticket {
  acknowledged_at: string | null
  area_id: string | null
  assignee_id: string | null
  assignee_name: string | null
  category: TicketCategory
  closed_at: string | null
  code: string
  created_at: string
  dedupe_key: string | null
  description: string
  device_slot: string | null
  due_at: string | null
  id: string
  last_seen_at: string
  metadata: Record<string, unknown>
  occurrence_count: number
  priority: TicketPriority
  reporter_id: string | null
  reporter_name: string | null
  resolution: string | null
  resolved_at: string | null
  room_id: string | null
  root_cause: 'hardware' | 'network' | 'software' | 'config' | 'usage' | 'other' | null
  serial: string | null
  source: TicketSource
  source_id: string | null
  source_type: string | null
  status: TicketStatus
  title: string
  updated_at: string
}

export interface TicketAssignee {
  display_name: string
  id: string
  roles: string[]
}

export interface TicketAssigneeStat {
  active: number
  assignee_id: string
  avg_resolve_secs: number | null
  display_name: string
  resolved: number
}

export interface TicketAttachment {
  content_type: string
  created_at: string
  download_url: string | null
  filename: string
  id: string
  object_key: string
  sha256: string
  size: number
  ticket_id: string
  uploaded_by: string | null
  uploaded_by_name: string | null
}

export interface TicketCategoryStat {
  category: TicketCategory
  count: number
}

export interface TicketContext {
  area_id: string | null
  area_name: string | null
  collected_at: string | null
  device_slot: string | null
  health_score: number | null
  health_status: 'ok' | 'warning' | 'critical' | null
  online: boolean | null
  room_code: string | null
  room_id: string | null
  room_name: string | null
  serial: string | null
}

export interface TicketDayStat {
  closed: number
  created: number
  date: string
  resolved: number
}

export interface TicketDetail {
  context: TicketContext
  events: TicketEvent[]
  ticket: Ticket
}

export interface TicketEvent {
  actor_id: string | null
  actor_name: string
  created_at: string
  from_status: TicketStatus | null
  id: number
  kind: TicketEventKind
  note: string
  payload: Record<string, unknown>
  source_id: string | null
  source_type: string | null
  ticket_id: string
  to_status: TicketStatus | null
}

export interface TicketPage {
  items: Ticket[]
  page: number
  page_size: number
  total: number
}

export interface TicketPriorityStat {
  count: number
  priority: TicketPriority
}

export interface TicketRule {
  created_at: string
  enabled: boolean
  id: string
  name: string
  priority: TicketPriority
  scope_id: string | null
  scope_type: 'global' | 'area' | 'room'
  signal_type:
    | 'machine_health'
    | 'device_offline'
    | 'schedule_delivery'
    | 'presence_delivery'
    | 'transfer'
    | 'image_verify'
  thresholds: Record<string, unknown>
  updated_at: string
}

export interface TicketRuleList {
  items: TicketRule[]
  total: number
}

export interface TicketScopeUser {
  area_ids: string[]
  display_name: string
  roles: string[]
  user_id: string
}

export interface TicketStats {
  by_assignee: TicketAssigneeStat[]
  by_category: TicketCategoryStat[]
  by_day: TicketDayStat[]
  by_priority: TicketPriorityStat[]
  from: string
  to: string
  top_devices: TicketTopDevice[]
  top_rooms: TicketTopRoom[]
  totals: TicketStatsTotals
}

export interface TicketStatsTotals {
  avg_resolve_secs: number | null
  closed: number
  created: number
  median_resolve_secs: number | null
  open_now: number
  overdue: number
  resolved: number
}

export interface TicketStreamEvent {
  actor_id: string | null
  actor_name: string
  created_at: string
  id: number
  kind: TicketEventKind
  note: string
  payload: Record<string, unknown>
  status: TicketStatus
  ticket_code: string
  ticket_id: string
}

export interface TicketSummary {
  by_status: Record<string, unknown>
  mine_active: number
  overdue: number
  unassigned: number
}

export interface TicketTopDevice {
  count: number
  device_slot: string | null
  serial: string
}

export interface TicketTopRoom {
  count: number
  room_id: string
  room_name: string
}

export interface TokenResponse {
  access_token: string
  expires_in: number
  refresh_token: string
  token_type: string
  user: UserProfile
}

export interface Transfer {
  created_at?: string
  direction?: 'download' | 'upload'
  error?: string | null
  id?: string
  image_id?: string | null
  manifest_digest?: string | null
  progress?: number
  room_id?: string
  status?: TransferStatus
  updated_at?: string
}

export interface TransferPage {
  items?: Transfer[]
  page?: number
  page_size?: number
  total?: number
}

export interface TransitionTicketRequest {
  note?: string | null
  resolution?: string | null
  root_cause?: 'hardware' | 'network' | 'software' | 'config' | 'usage' | 'other' | null
  to_status: TicketStatus
}

export interface UpdateAgentConfigReply {
  config: AgentConfig
  pushed_rooms: number
}

export interface UpdateAgentConfigRequest {
  policy: AgentPolicyPatch
  scope_id?: string | null
  scope_type: AgentConfigScopeType
}

export interface UpdateAreaRequest {
  code?: string
  description?: string
  is_active?: boolean
  metadata?: Record<string, unknown> | null
  name?: string
  parent_id?: string | null
  timezone?: string
  webhook_url?: string | null
}

export interface UpdateClassRequest {
  code?: string
  grade?: string
  is_active?: boolean
  metadata?: Record<string, unknown> | null
  name?: string
}

export interface UpdateCourseRequest {
  code?: string
  color?: string
  is_active?: boolean
  metadata?: Record<string, unknown> | null
  name?: string
}

export interface UpdateInspectionPlanRequest {
  enabled?: boolean
  interval_hours?: number
  name?: string
  scope_id?: string | null
  scope_type?: InspectionScopeType
}

export interface UpdateRoomRequest {
  area_id?: string
  capacity?: number
  code?: string
  device_slot?: string | null
  is_active?: boolean
  metadata?: Record<string, unknown> | null
  name?: string
  webhook_url?: string | null
}

export interface UpdateSchedulePlanRequest {
  name: string
}

export interface UpdateScheduleRequest {
  end_date?: string
  is_active?: boolean
  name?: string
  start_date?: string
}

export interface UpdateTeacherRequest {
  code?: string
  email?: string
  is_active?: boolean
  metadata?: Record<string, unknown> | null
  name?: string
  phone?: string
}

export interface UpdateTermRequest {
  code?: string
  end_date?: string
  is_active?: boolean
  name?: string
  start_date?: string
}

export interface UpdateTicketRequest {
  category?: TicketCategory
  description?: string | null
  priority?: TicketPriority
  title?: string
}

export interface UpdateTicketRuleRequest {
  enabled?: boolean
  name?: string
  priority?: TicketPriority
  scope_id?: string | null
  scope_type?: 'global' | 'area' | 'room'
  thresholds?: Record<string, unknown>
}

export interface UpdateTicketScopesRequest {
  area_ids: string[]
}

export interface UpdateUserRequest {
  display_name?: string
  email?: string
  is_active?: boolean
  password?: string
}

export interface UpsertExceptionRequest {
  kind: 'cancel' | 'replace'
  note?: string
  slots?: NewExceptionSlot[]
}

export interface UserPage {
  items?: UserSummary[]
  page?: number
  page_size?: number
  total?: number
}

export interface UserProfile {
  created_at?: string
  display_name: string
  email?: string | null
  id: string
  is_active: boolean
  permissions: string[]
  roles: string[]
  username: string
}

export interface UserSession {
  created_at?: string
  id?: string
  ip?: string | null
  last_seen_at?: string
  revoked_at?: string | null
  user_agent?: string
  user_id?: string
}

export interface UserSummary {
  created_at?: string
  display_name?: string
  email?: string | null
  id?: string
  is_active?: boolean
  roles?: string[]
  updated_at?: string
  username?: string
}

export interface VerifyReply {
  artifact: ImageArtifact
  verification: ImageVerification
}

export interface WeeklySlot {
  course_name?: string
  created_at?: string
  day_of_week?: number
  end_time?: string
  id?: string
  note?: string
  room_id?: string | null
  room_name?: string | null
  schedule_id?: string
  start_time?: string
  teacher?: string
}
