# Backend API contract (generated from live /api-docs/openapi.json)

## GET /admin/agent-configs
- summary: 所有作用域的节点配置（global / area / room）
- tags: AgentConfigs
- response 200 application/json: AgentConfigList
- response 403 application/json: Error

## PUT /admin/agent-configs
- summary: 写入作用域配置（upsert 后审计并二次下发 config.update）
- desc: global 的 scope_id 必须为空；area/room 必须提供且目标存在。policy 为局部 patch，先做范围校验。
- tags: AgentConfigs
- requestBody application/json: UpdateAgentConfigRequest
- response 200 application/json: UpdateAgentConfigReply
- response 400 application/json: Error
- response 403 application/json: Error
- response 404 application/json: Error

## GET /admin/agent-configs/effective
- summary: 某房间合并后的生效策略（global → area → room）
- tags: AgentConfigs
- params:
  - `room_id` in=query req=True type=string — 
- response 200 application/json: EffectiveAgentPolicyReply
- response 403 application/json: Error
- response 404 application/json: Error

## GET /admin/announcements
- summary: 后台公告列表
- tags: Admin
- params:
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
  - `keyword` in=query req=False type=string — 
  - `status` in=query req=False type="draft" | "published" | "archived" — 
- response 200 application/json: AnnouncementPage
- response 403 application/json: Error

## POST /admin/announcements
- summary: 创建公告
- tags: Admin
- requestBody application/json: AnnouncementInput
- response 201 application/json: Announcement
- response 403 application/json: Error

## DELETE /admin/announcements/{id}
- summary: 删除公告
- tags: Admin
- params:
  - `id` in=path req=True type=string — 
- response 204: 已删除

## PATCH /admin/announcements/{id}
- summary: 更新公告
- tags: Admin
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: AnnouncementInput
- response 200 application/json: Announcement

## GET /admin/areas
- summary: 区域分页
- tags: Areas
- params:
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
  - `keyword` in=query req=False type=string — 
  - `include_inactive` in=query req=False type=boolean — 
- response 200 application/json: AreaPage
- response 403 application/json: Error

## POST /admin/areas
- summary: 创建区域（可选 IANA timezone，缺省 Asia/Shanghai）
- tags: Areas
- requestBody application/json: CreateAreaRequest
- response 201 application/json: Area
- response 400 application/json: Error
- response 403 application/json: Error
- response 409 application/json: Error

## DELETE /admin/areas/{id}
- summary: 删除区域（仍有房间时 409）
- tags: Areas
- params:
  - `id` in=path req=True type=string — 
- response 204: 已删除
- response 404 application/json: Error
- response 409 application/json: Error

## GET /admin/areas/{id}
- summary: 区域详情
- tags: Areas
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: Area
- response 404 application/json: Error

## PATCH /admin/areas/{id}
- summary: 局部更新区域（parent_id / metadata 可传 null；timezone 变更后重建区域实例）
- tags: Areas
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: UpdateAreaRequest
- response 200 application/json: Area
- response 400 application/json: Error
- response 404 application/json: Error

## GET /admin/audit-logs
- summary: 审计日志
- tags: Admin
- params:
  - `action` in=query req=False type=string — 
  - `resource_type` in=query req=False type=string — 
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
- response 200 application/json: AuditLogPage
- response 403 application/json: Error

## GET /admin/collections
- summary: 收集登记分页（按房间过滤，created_at 倒序）
- tags: Storage
- params:
  - `room_id` in=query req=False type=string — 
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
- response 200 application/json: CollectionPage
- response 403 application/json: Error

## GET /admin/device-events
- summary: 设备在线事件（id 倒序；limit 1–200，默认 50）
- tags: DeviceEvents
- params:
  - `limit` in=query req=False type=number — 
  - `after_id` in=query req=False type=number — 只返回 id > after_id 的事件。
- response 200 application/json: PresenceEventList
- response 400 application/json: Error
- response 403 application/json: Error

## GET /admin/device-events/stream
- summary: 设备在线事件 SSE 流（先补发历史，之后每 2s 增量 + 15s 心跳）
- desc: 事件类型 `device.presence`，data 为 PresenceEvent JSON；心跳事件类型 `heartbeat`。跨副本以查库为准。
- tags: DeviceEvents
- params:
  - `after_id` in=query req=False type=number — 从该事件 id 之后开始补发；缺省返回最近 200 条。
- response 200 text/event-stream: string
- response 400 application/json: Error
- response 403 application/json: Error

## GET /admin/device-presence-deliveries
- summary: 设备在线事件投递记录（id 倒序，排障用）
- tags: DeviceEvents
- params:
  - `status` in=query req=False type="pending" | "delivered" | "failed" — 
  - `limit` in=query req=False type=number — 
- response 200 application/json: PresenceDeliveryList
- response 400 application/json: Error
- response 403 application/json: Error

## GET /admin/devices
- summary: 设备分页（含房间/区域、注册状态与在线状态）
- tags: Devices
- params:
  - `area_id` in=query req=False type=string — 
  - `room_id` in=query req=False type=string — 
  - `status` in=query req=False type=DeviceStatus — 
  - `assigned` in=query req=False type=boolean — true = 只看已绑定房间；false = 只看未绑定。
  - `keyword` in=query req=False type=string — 模糊匹配 serial / device_slot / 房间编码 / 房间名称。
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
- response 200 application/json: DevicePage
- response 400 application/json: Error
- response 403 application/json: Error

## GET /admin/devices/{serial}
- summary: 设备详情
- tags: Devices
- params:
  - `serial` in=path req=True type=string — 
- response 200 application/json: DeviceInfo
- response 403 application/json: Error
- response 404 application/json: Error

## GET /admin/images
- summary: 镜像分页（kind / status / name）
- tags: Images
- params:
  - `kind` in=query req=False type=ImageKind — 
  - `status` in=query req=False type=ArtifactStatus — 
  - `name` in=query req=False type=string — 按名称模糊匹配。
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
- response 200 application/json: ImagePage
- response 400 application/json: Error
- response 403 application/json: Error

## POST /admin/images
- summary: 创建镜像制品（uploaded；name+version+platform 唯一）
- tags: Images
- requestBody application/json: CreateImageRequest
- response 201 application/json: ImageArtifact
- response 400 application/json: Error
- response 403 application/json: Error
- response 409 application/json: Error

## GET /admin/images/{id}
- summary: 镜像详情 + 分片 + 最近验证报告 + 已发布通道
- tags: Images
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: ImageDetail
- response 403 application/json: Error
- response 404 application/json: Error

## GET /admin/images/{id}/manifest
- summary: 读取 manifest JSON + 预签名原始下载地址
- tags: Images
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: ManifestReply
- response 403 application/json: Error
- response 404 application/json: Error
- response 409 application/json: Error
- response 503 application/json: Error

## POST /admin/images/{id}/manifest
- summary: 提交 manifest：校验分片对象、签名并写入 S3，状态置 manifest_ready
- tags: Images
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: SubmitManifestRequest
- response 200 application/json: SubmitManifestReply
- response 400 application/json: Error
- response 403 application/json: Error
- response 404 application/json: Error
- response 409 application/json: Error
- response 503 application/json: Error

## POST /admin/images/{id}/publish
- summary: 发布到通道（dev / test / prod），仅 verified 可发布
- tags: Images
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: PublishImageRequest
- response 200 application/json: ImageArtifact
- response 400 application/json: Error
- response 403 application/json: Error
- response 404 application/json: Error
- response 409 application/json: Error

## POST /admin/images/{id}/verify
- summary: 同步验证：分片 sha256/size、总大小、格式 magic、manifest 签名；写验证报告
- tags: Images
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: VerifyReply
- response 400 application/json: Error
- response 403 application/json: Error
- response 404 application/json: Error
- response 409 application/json: Error
- response 503 application/json: Error

## GET /admin/machines
- summary: 机器分页（设备 + 房间/区域 + 最新快照摘要）
- tags: Machines
- params:
  - `area_id` in=query req=False type=string — 
  - `room_id` in=query req=False type=string — 
  - `keyword` in=query req=False type=string — serial / 槽位 / 房间 code/name / hostname 模糊匹配
  - `health` in=query req=False type="ok" | "warning" | "critical" | "missing" | "unknown" — missing = 从未上报
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
- response 200 application/json: MachinePage
- response 400 application/json: Error
- response 403 application/json: Error

## GET /admin/machines/stream
- summary: 机器快照 SSE 实时流（新快照 2s 内推送）
- desc: 首次连接从当前最新快照 id 开始（不补历史）；断线重连时带最后收到的 id 作为 after_id，可补发最多 200 条。帧事件：`ready`（建连即发，data 为空）、`machine.snapshot`（data 为 MachineSnapshotPoint）、每 15s `heartbeat`。浏览器 EventSource 不能带 Authorization，请用 fetch + ReadableStream。
- tags: Machines
- params:
  - `after_id` in=query req=False type=number — 从该快照 id 之后补发；缺省 = 只推送连接之后的新快照。
  - `serial` in=query req=False type=string — 只订阅某台机器；缺省为全部机器。
- response 200 text/event-stream: string
- response 400 application/json: Error
- response 403 application/json: Error

## GET /admin/machines/{serial}
- summary: 机器详情（设备 + 最新完整快照 + 历史）
- tags: Machines
- params:
  - `serial` in=path req=True type=string — 
  - `history_limit` in=query req=False type=number — 
- response 200 application/json: MachineDetail
- response 403 application/json: Error
- response 404 application/json: Error

## GET /admin/machines/{serial}/snapshots
- summary: 机器快照历史（元数据，collected_at 倒序）
- tags: Machines
- params:
  - `serial` in=path req=True type=string — 
  - `limit` in=query req=False type=number — 
- response 200 application/json: MachineSnapshotPoint[]
- response 403 application/json: Error
- response 404 application/json: Error

## GET /admin/permissions
- summary: 权限列表
- tags: Admin
- response 200 application/json: Permission[]

## GET /admin/roles
- summary: 角色列表
- tags: Admin
- response 200 application/json: RoleSummary[]

## POST /admin/roles
- summary: 创建角色
- tags: Admin
- requestBody application/json: CreateRoleRequest
- response 201 application/json: RoleSummary

## DELETE /admin/roles/{id}
- summary: 删除角色
- tags: Admin
- params:
  - `id` in=path req=True type=string — 
- response 204: 已删除

## PUT /admin/roles/{id}/permissions
- summary: 设置角色权限
- tags: Admin
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: SetPermissionsRequest
- response 200 application/json: RoleSummary

## GET /admin/rooms
- summary: 房间分页
- tags: Rooms
- params:
  - `area_id` in=query req=False type=string — 
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
  - `keyword` in=query req=False type=string — 
  - `include_inactive` in=query req=False type=boolean — 
- response 200 application/json: RoomPage
- response 403 application/json: Error

## POST /admin/rooms
- summary: 创建房间
- tags: Rooms
- requestBody application/json: CreateRoomRequest
- response 201 application/json: Room
- response 400 application/json: Error
- response 403 application/json: Error
- response 409 application/json: Error

## DELETE /admin/rooms/{id}
- summary: 删除房间（级联删除课表与触发规则）
- tags: Rooms
- params:
  - `id` in=path req=True type=string — 
- response 204: 已删除
- response 404 application/json: Error

## GET /admin/rooms/{id}
- summary: 房间详情
- tags: Rooms
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: Room
- response 404 application/json: Error

## PATCH /admin/rooms/{id}
- summary: 局部更新房间（device_slot / webhook_url 可传 null 清空）
- tags: Rooms
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: UpdateRoomRequest
- response 200 application/json: Room
- response 400 application/json: Error
- response 404 application/json: Error

## GET /admin/rooms/{id}/device
- summary: 房间设备摘要（未绑定槽位返回 null）
- tags: Devices
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: RoomDevice | null
- response 403 application/json: Error
- response 404 application/json: Error

## PUT /admin/rooms/{id}/device
- summary: 绑定 / 迁移 / 解绑房间设备槽位（同一槽位自动从旧房间迁移）
- tags: Devices
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: { device_slot: string | null }
- response 200 application/json: Room
- response 400 application/json: Error
- response 403 application/json: Error
- response 404 application/json: Error
- response 409 application/json: Error

## POST /admin/rooms/{id}/device-token
- summary: 签发设备注册 token（房间无槽位时自动分配；明文 token 只返回一次）
- tags: Devices
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: DeviceTokenRequest
- response 200 application/json: DeviceTokenReply
- response 400 application/json: Error
- response 403 application/json: Error
- response 404 application/json: Error

## GET /admin/rooms/{id}/event-rules
- summary: 房间触发规则
- tags: ScheduleEvents
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: EventRule[]
- response 404 application/json: Error

## PUT /admin/rooms/{id}/event-rules
- summary: 全量替换房间触发规则
- tags: ScheduleEvents
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: ReplaceEventRulesRequest
- response 200 application/json: EventRule[]
- response 400 application/json: Error
- response 403 application/json: Error
- response 404 application/json: Error

## DELETE /admin/rooms/{id}/images/{image_id}
- summary: 取消房间分配
- tags: Images
- params:
  - `id` in=path req=True type=string — 
  - `image_id` in=path req=True type=string — 
- response 204: 已取消分配
- response 403 application/json: Error
- response 404 application/json: Error

## PUT /admin/rooms/{id}/images/{image_id}
- summary: 把已发布镜像分配到房间（创建 download transfer）
- tags: Images
- params:
  - `id` in=path req=True type=string — 
  - `image_id` in=path req=True type=string — 
- requestBody application/json: AssignImageRequest
- response 200 application/json: ImageAssignment
- response 400 application/json: Error
- response 403 application/json: Error
- response 404 application/json: Error
- response 409 application/json: Error

## GET /admin/rooms/{id}/occurrences
- summary: 房间维度排课实例（UTC + 区域时区本地时间；默认按房间时区今天起 7 天）
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
  - `from` in=query req=False type=string — 
  - `to` in=query req=False type=string — 
- response 200 application/json: Occurrence[]
- response 400 application/json: Error
- response 404 application/json: Error

## GET /admin/schedule-events
- summary: 事件分页（triggered_at 倒序；含 timezone / triggered_at_local）
- tags: ScheduleEvents
- params:
  - `room_id` in=query req=False type=string — 
  - `kind` in=query req=False type="occurrence_start" | "occurrence_end" | "occurrence_start_before" | "daily_time" | "room_occupied" | "room_idle" — 
  - `from` in=query req=False type=string — 
  - `to` in=query req=False type=string — 
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
- response 200 application/json: ScheduleEventPage
- response 400 application/json: Error
- response 403 application/json: Error

## GET /admin/schedule-events/{id}
- summary: 事件 + 投递明细
- tags: ScheduleEvents
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: ScheduleEventDetail
- response 404 application/json: Error

## POST /admin/schedule-events/{id}/deliveries/{delivery_id}/retry
- summary: 单条投递立即重试
- tags: ScheduleEvents
- params:
  - `id` in=path req=True type=string — 
  - `delivery_id` in=path req=True type=number — 
- response 204: 已重置为待投递
- response 403 application/json: Error
- response 404 application/json: Error

## POST /admin/schedule-events/{id}/replay
- summary: 把事件全部投递重置为 pending
- tags: ScheduleEvents
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: ReplayResult
- response 403 application/json: Error
- response 404 application/json: Error

## GET /admin/schedules
- summary: 课表分页
- tags: Schedules
- params:
  - `room_id` in=query req=False type=string — 
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
- response 200 application/json: SchedulePage
- response 403 application/json: Error

## POST /admin/schedules
- summary: 创建课表
- tags: Schedules
- requestBody application/json: CreateScheduleRequest
- response 201 application/json: Schedule
- response 400 application/json: Error
- response 403 application/json: Error

## DELETE /admin/schedules/{id}
- summary: 删除课表并级联删除实例
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
- response 204: 已删除
- response 404 application/json: Error

## GET /admin/schedules/{id}
- summary: 课表详情
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: Schedule
- response 404 application/json: Error

## PATCH /admin/schedules/{id}
- summary: 局部更新课表；成功后重建未来排课实例
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: UpdateScheduleRequest
- response 200 application/json: Schedule
- response 400 application/json: Error
- response 404 application/json: Error
- response 500 application/json: Error

## GET /admin/schedules/{id}/exceptions
- summary: 单日覆盖列表
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
  - `from` in=query req=False type=string — 
  - `to` in=query req=False type=string — 
- response 200 application/json: ScheduleException[]
- response 404 application/json: Error

## DELETE /admin/schedules/{id}/exceptions/{date}
- summary: 删除单日覆盖并重建未来排课实例
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
  - `date` in=path req=True type=string — 
- response 204: 已删除
- response 404 application/json: Error
- response 500 application/json: Error

## PUT /admin/schedules/{id}/exceptions/{date}
- summary: upsert 单日覆盖（cancel / replace）；成功后重建未来排课实例
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
  - `date` in=path req=True type=string — 
- requestBody application/json: UpsertExceptionRequest
- response 200 application/json: ScheduleException
- response 400 application/json: Error
- response 404 application/json: Error
- response 500 application/json: Error

## GET /admin/schedules/{id}/occurrences
- summary: 排课实例（UTC start_at/end_at + 区域时区 start_local/end_local；默认按区域时区今天起 7 天）
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
  - `from` in=query req=False type=string — 
  - `to` in=query req=False type=string — 
- response 200 application/json: Occurrence[]
- response 400 application/json: Error
- response 404 application/json: Error

## POST /admin/schedules/{id}/rebuild
- summary: 手动重建未来排课实例
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: RebuildResult
- response 403 application/json: Error
- response 404 application/json: Error
- response 500 application/json: Error

## GET /admin/schedules/{id}/weekly-slots
- summary: 周模板
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: WeeklySlot[]
- response 404 application/json: Error

## PUT /admin/schedules/{id}/weekly-slots
- summary: 全量替换周模板；成功后重建未来排课实例
- tags: Schedules
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: ReplaceWeeklySlotsRequest
- response 200 application/json: WeeklySlot[]
- response 400 application/json: Error
- response 404 application/json: Error
- response 500 application/json: Error

## POST /admin/storage/objects
- summary: 登记已上传对象（服务端从 S3 读回并重算 sha256/size，不一致 400）
- tags: Storage
- requestBody application/json: StorageObjectRequest
- response 201 application/json: StorageObject
- response 400 application/json: Error
- response 403 application/json: Error
- response 503 application/json: Error

## POST /admin/storage/objects/presign
- summary: 初始化对象上传：返回 objects/{sha256} 的短 TTL 预签名 PUT
- tags: Storage
- requestBody application/json: StorageObjectRequest
- response 200 application/json: PresignObjectReply
- response 400 application/json: Error
- response 403 application/json: Error
- response 503 application/json: Error

## GET /admin/storage/objects/{sha256}
- summary: 对象详情 + 预签名下载地址
- tags: Storage
- params:
  - `sha256` in=path req=True type=string — 
- response 200 application/json: StorageObjectReply
- response 400 application/json: Error
- response 403 application/json: Error
- response 404 application/json: Error
- response 503 application/json: Error

## GET /admin/transfers
- summary: 分发/收集任务分页（按房间过滤，created_at 倒序）
- tags: Storage
- params:
  - `room_id` in=query req=False type=string — 
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
- response 200 application/json: TransferPage
- response 403 application/json: Error

## POST /admin/transfers/{id}/retry
- summary: 重试分发任务（failed / expired → pending，进度清零）
- tags: Storage
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: Transfer
- response 403 application/json: Error
- response 404 application/json: Error
- response 409 application/json: Error

## GET /admin/users
- summary: 用户列表
- tags: Admin
- params:
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
  - `keyword` in=query req=False type=string — 
- response 200 application/json: UserPage

## POST /admin/users
- summary: 创建用户
- tags: Admin
- requestBody application/json: CreateUserRequest
- response 201 application/json: UserSummary

## PATCH /admin/users/{id}
- summary: 更新用户
- tags: Admin
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: UpdateUserRequest
- response 200 application/json: UserSummary

## PUT /admin/users/{id}/roles
- summary: 设置用户角色
- tags: Admin
- params:
  - `id` in=path req=True type=string — 
- requestBody application/json: SetRolesRequest
- response 200 application/json: UserSummary

## GET /announcements
- summary: 公开公告列表
- tags: Announcements
- params:
  - `page` in=query req=False type=number — 
  - `page_size` in=query req=False type=number — 
  - `keyword` in=query req=False type=string — 
- response 200 application/json: AnnouncementPage

## GET /announcements/{id}
- summary: 公开公告详情
- tags: Announcements
- params:
  - `id` in=path req=True type=string — 
- response 200 application/json: Announcement
- response 404 application/json: Error

## POST /auth/email/verification
- summary: 请求邮箱验证邮件
- tags: Auth
- response 204: 验证邮件已发送（开发环境写日志）
- response 400 application/json: Error

## POST /auth/email/verify
- summary: 验证邮箱
- tags: Auth
- requestBody application/json: { token: string }
- response 204: 邮箱已验证
- response 400 application/json: Error

## POST /auth/login
- summary: 登录
- tags: Auth
- requestBody application/json: LoginRequest
- response 200 application/json: TokenResponse
- response 401 application/json: Error
- response 429 application/json: Error

## POST /auth/logout
- summary: 注销 refresh token
- tags: Auth
- requestBody application/json: RefreshRequest
- response 204: 已注销

## GET /auth/me
- summary: 当前用户
- tags: Auth
- response 200 application/json: UserProfile
- response 401 application/json: Error

## POST /auth/password/forgot
- summary: 请求密码重置邮件
- tags: Auth
- requestBody application/json: { email: string }
- response 204: 已受理；无论邮箱是否存在都返回 204

## POST /auth/password/reset
- summary: 使用一次性令牌重置密码
- tags: Auth
- requestBody application/json: { new_password: string; token: string }
- response 204: 密码已重置，所有设备会话已撤销
- response 400 application/json: Error

## POST /auth/refresh
- summary: 轮换 refresh token
- tags: Auth
- requestBody application/json: RefreshRequest
- response 200 application/json: TokenResponse
- response 401 application/json: Error

## DELETE /auth/sessions
- summary: 撤销当前用户全部设备会话
- tags: Auth
- response 204: 已撤销

## GET /auth/sessions
- summary: 列出当前用户的设备会话
- tags: Auth
- response 200 application/json: UserSession[]

## DELETE /auth/sessions/{id}
- summary: 撤销指定设备会话
- tags: Auth
- params:
  - `id` in=path req=True type=string — 
- response 204: 已撤销
- response 404 application/json: Error

## POST /machine/report
- summary: room-agent 上报机器快照（fleet Bearer token）
- desc: 认证头：Authorization: Bearer <MACHINE_AGENT_TOKEN>；服务端未配置 token 时返回 503。按 device_slot 解析已注册设备（issued/active），同一 (serial, collected_at) 幂等。
- tags: MachineAgent
- requestBody application/json: MachineReportRequest
- response 202 application/json: MachineReportReply
- response 400 application/json: Error
- response 401 application/json: Error
- response 404 application/json: Error
- response 503 application/json: Error
