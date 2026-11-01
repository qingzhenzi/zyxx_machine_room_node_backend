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

export interface CreateImageRequest {
  compression?: Compression
  format: ImageFormat
  kind: ImageKind
  name: string
  platform: string
  version: string
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

export interface CreateScheduleRequest {
  end_date: string
  is_active?: boolean
  name: string
  room_id: string
  start_date: string
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

export interface RebuildResult {
  occurrences?: number
}

export interface RefreshRequest {
  refresh_token: string
}

export interface ReplaceEventRulesRequest {
  rules: NewEventRule[]
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

export interface UpdateScheduleRequest {
  end_date?: string
  is_active?: boolean
  name?: string
  start_date?: string
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
