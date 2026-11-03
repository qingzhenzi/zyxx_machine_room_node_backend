use axum::{Json, response::Html};
use serde_json::{Value, json};

const DOCS_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>QEStudio API</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css" />
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
  <script>
    window.ui = SwaggerUIBundle({
      url: '/api-docs/openapi.json',
      dom_id: '#swagger-ui',
      deepLinking: true,
      persistAuthorization: true,
    });
  </script>
</body>
</html>"#;

fn error_response(description: &str) -> Value {
    json!({
        "description": description,
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Error" } } }
    })
}

fn secured() -> Value {
    json!([{ "bearerAuth": [] }])
}

/// 构建 OpenAPI 文档（纯函数；HTTP 端点与离线导出示例共用）。
pub fn spec() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "QEStudio API",
            "version": "0.1.0",
            "description": "QEStudio 后端 API。认证使用 Bearer access token；管理接口需要对应 RBAC 权限。"
        },
        "servers": [{ "url": "/api/v1" }],
        "tags": [
            { "name": "Auth" },
            { "name": "Announcements" },
            { "name": "Admin" },
            { "name": "Areas", "description": "区域：楼栋 / 楼层 / 片区（总管理节点预留 parent_id）" },
            { "name": "Rooms", "description": "房间：调度最小单位，可绑定设备槽位" },
            { "name": "Schedules", "description": "课表：周模板、单日覆盖与排课实例" },
            { "name": "ScheduleBoard", "description": "排课工作台：区域/房间/课表/周模板/单日覆盖一次聚合，并为房间幂等补默认课表" },
            { "name": "Terms", "description": "学期：排课计划归属的起止区间与启用状态" },
            { "name": "Teachers", "description": "排课主数据：教师（轻量编码 + 名称 / 联系方式）" },
            { "name": "Classes", "description": "排课主数据：班级（轻量编码 + 名称 / 年级）" },
            { "name": "Courses", "description": "排课主数据：课程（轻量编码 + 名称 / 色值）" },
            { "name": "SchedulePlans", "description": "排课计划：草稿 / 发布、条目编辑与发布桥接写入 legacy 课表（路径 2 Phase A）" },
            { "name": "ScheduleEvents", "description": "触发规则、事件与投递重放/重试" },
            { "name": "Devices", "description": "设备：注册状态、在线状态与区域/房间绑定（v3）" },
            { "name": "DeviceEvents", "description": "设备在线事件：历史查询、SSE 实时流与可靠投递记录（P3）" },
            { "name": "TicketInspections", "description": "巡检计划：周期检查 scope 内机器健康，异常自动建工单" },
            { "name": "Tickets", "description": "工单：人工/自动建单、状态机、时间线、统计与「我的待办」（P0）" },
            { "name": "Machines", "description": "机器总览：room-agent 周期性上报的主机清单、健康结论与诊断历史" },
            { "name": "MachineAgent", "description": "机器遥测上报端点（room-agent 使用 fleet Bearer token）" },
            { "name": "Storage", "description": "对象存储：预签名上传、对象登记、分发/收集任务（P0，S3 兼容）" },
            { "name": "Images", "description": "镜像：manifest、验证、发布与房间分配（P0，不做 Registry 协议）" },
            { "name": "AgentConfigs", "description": "节点配置：按 global → area → room 合并的生效策略与二次下发（P2）" },
            { "name": "Health" }
        ],
        "components": {
            "securitySchemes": {
                "bearerAuth": { "type": "http", "scheme": "bearer", "bearerFormat": "JWT" }
            },
            "schemas": {
                "Error": {
                    "type": "object",
                    "required": ["error"],
                    "properties": {
                        "error": {
                            "type": "object",
                            "required": ["code", "message"],
                            "properties": {
                                "code": { "type": "string" },
                                "message": { "type": "string" }
                            }
                        }
                    }
                },
                "LoginRequest": {
                    "type": "object",
                    "required": ["username", "password"],
                    "properties": {
                        "username": { "type": "string" },
                        "password": { "type": "string", "format": "password" }
                    }
                },
                "RefreshRequest": {
                    "type": "object",
                    "required": ["refresh_token"],
                    "properties": { "refresh_token": { "type": "string" } }
                },
                "UserProfile": {
                    "type": "object",
                    "required": ["id", "username", "display_name", "is_active", "roles", "permissions"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "username": { "type": "string" },
                        "email": { "type": ["string", "null"] },
                        "display_name": { "type": "string" },
                        "is_active": { "type": "boolean" },
                        "roles": { "type": "array", "items": { "type": "string" } },
                        "permissions": { "type": "array", "items": { "type": "string" } },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "TokenResponse": {
                    "type": "object",
                    "required": ["token_type", "access_token", "refresh_token", "expires_in", "user"],
                    "properties": {
                        "token_type": { "type": "string", "const": "Bearer" },
                        "access_token": { "type": "string" },
                        "refresh_token": { "type": "string" },
                        "expires_in": { "type": "integer" },
                        "user": { "$ref": "#/components/schemas/UserProfile" }
                    }
                },
                "Announcement": {
                    "type": "object",
                    "required": ["id", "title", "content", "status", "is_pinned", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "title": { "type": "string" },
                        "content": { "type": "string", "description": "写入时已由服务端做 HTML 清洗；前端仍应再次消毒。" },
                        "status": { "type": "string", "enum": ["draft", "published", "archived"] },
                        "is_pinned": { "type": "boolean" },
                        "author_id": { "type": ["string", "null"], "format": "uuid" },
                        "author_name": { "type": ["string", "null"] },
                        "published_at": { "type": ["string", "null"], "format": "date-time" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "AnnouncementPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/Announcement" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "AnnouncementInput": {
                    "type": "object",
                    "required": ["title", "content"],
                    "properties": {
                        "title": { "type": "string", "maxLength": 200 },
                        "content": { "type": "string", "maxLength": 100000 },
                        "status": { "type": "string", "enum": ["draft", "published", "archived"] },
                        "is_pinned": { "type": "boolean" }
                    }
                },
                "UserSummary": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "username": { "type": "string" },
                        "email": { "type": ["string", "null"] },
                        "display_name": { "type": "string" },
                        "is_active": { "type": "boolean" },
                        "roles": { "type": "array", "items": { "type": "string" } },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "UserPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/UserSummary" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "CreateUserRequest": {
                    "type": "object",
                    "required": ["username", "password", "display_name"],
                    "properties": {
                        "username": { "type": "string" },
                        "password": { "type": "string", "format": "password" },
                        "display_name": { "type": "string" },
                        "email": { "type": "string" },
                        "role_codes": { "type": "array", "items": { "type": "string" } }
                    }
                },
                "UpdateUserRequest": {
                    "type": "object",
                    "properties": {
                        "display_name": { "type": "string" },
                        "email": { "type": "string" },
                        "is_active": { "type": "boolean" },
                        "password": { "type": "string", "format": "password" }
                    }
                },
                "SetRolesRequest": {
                    "type": "object",
                    "properties": { "role_codes": { "type": "array", "items": { "type": "string" } } }
                },
                "RoleSummary": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "is_system": { "type": "boolean" },
                        "permissions": { "type": "array", "items": { "type": "string" } }
                    }
                },
                "CreateRoleRequest": {
                    "type": "object",
                    "required": ["code", "name"],
                    "properties": {
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "permission_codes": { "type": "array", "items": { "type": "string" } }
                    }
                },
                "SetPermissionsRequest": {
                    "type": "object",
                    "properties": { "permission_codes": { "type": "array", "items": { "type": "string" } } }
                },
                "Permission": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" }
                    }
                },
                "UserSession": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "user_id": { "type": "string", "format": "uuid" },
                        "user_agent": { "type": "string" },
                        "ip": { "type": ["string", "null"] },
                        "created_at": { "type": "string", "format": "date-time" },
                        "last_seen_at": { "type": "string", "format": "date-time" },
                        "revoked_at": { "type": ["string", "null"], "format": "date-time" }
                    }
                },
                "AuditLog": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "actor_id": { "type": ["string", "null"], "format": "uuid" },
                        "actor_username": { "type": "string" },
                        "actor_display_name": { "type": "string" },
                        "action": { "type": "string" },
                        "resource_type": { "type": "string" },
                        "resource_id": { "type": ["string", "null"] },
                        "detail": { "type": "object" },
                        "ip": { "type": ["string", "null"] },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "AuditLogPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/AuditLog" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "Area": {
                    "type": "object",
                    "required": ["id", "code", "name", "description", "is_active", "metadata", "timezone", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "parent_id": { "type": ["string", "null"], "format": "uuid", "description": "总管理节点 / 子管理节点预留，本版本不做节点间同步。" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": "object" },
                        "timezone": { "type": "string", "example": "Asia/Shanghai", "description": "IANA 时区；房间/课表继承该时区，所有时间点以 UTC 存储、按此时区展示本地时间。" },
                        "webhook_url": { "type": ["string", "null"], "format": "uri", "description": "区域级设备在线事件 webhook；房间未配置时使用，再回退到全局配置。" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "AreaPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/Area" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "CreateAreaRequest": {
                    "type": "object",
                    "required": ["code", "name"],
                    "properties": {
                        "code": { "type": "string", "description": "2-64 位，首字符字母，可含字母数字 - _ ." },
                        "name": { "type": "string", "maxLength": 200 },
                        "description": { "type": "string" },
                        "parent_id": { "type": ["string", "null"], "format": "uuid" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": "object" },
                        "timezone": { "type": "string", "example": "Asia/Shanghai", "description": "IANA 时区名，缺省 Asia/Shanghai。" },
                        "webhook_url": { "type": ["string", "null"], "format": "uri", "description": "区域级设备在线事件 webhook；空字符串表示不配置。" }
                    }
                },
                "UpdateAreaRequest": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "parent_id": { "type": ["string", "null"], "format": "uuid", "description": "缺失 = 不修改；null = 清空。" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": ["object", "null"] },
                        "timezone": { "type": "string", "description": "修改后按新时区自动重建该区域下所有课表的未来排课实例。" },
                        "webhook_url": { "type": ["string", "null"], "description": "缺失 = 不修改；null = 清空。" }
                    }
                },
                "Room": {
                    "type": "object",
                    "required": ["id", "area_id", "area_name", "code", "name", "capacity", "device", "timezone", "is_active", "metadata", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "area_id": { "type": "string", "format": "uuid" },
                        "area_name": { "type": "string" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "capacity": { "type": "integer", "minimum": 0 },
                        "device_slot": { "type": ["string", "null"], "description": "绑定 device_enrollment.device_slot；唯一。" },
                        "device": { "oneOf": [{ "$ref": "#/components/schemas/RoomDevice" }, { "type": "null" }], "description": "只读：房间绑定槽位后的设备注册/在线摘要；null = 未绑定。" },
                        "webhook_url": { "type": ["string", "null"], "format": "uri" },
                        "timezone": { "type": "string", "readOnly": true, "example": "Asia/Shanghai", "description": "只读：继承自所属区域。" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": "object" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "RoomPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/Room" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "DeviceStatus": {
                    "type": "string",
                    "enum": ["pending", "issued", "active", "rejected", "revoked"],
                    "description": "设备注册状态（来自 device_enrollment.status）。"
                },
                "DeviceOnlineState": {
                    "type": "string",
                    "enum": ["online", "offline"],
                    "description": "设备在线状态：device-pki 会话为 online，或最近 5 分钟内有 room-agent 快照。"
                },
                "RoomDevice": {
                    "type": "object",
                    "required": ["device_slot", "serial", "status", "online", "last_seen_at", "pending_token"],
                    "properties": {
                        "device_slot": { "type": "string" },
                        "serial": { "type": ["string", "null"], "description": "null = 已分配槽位但尚未注册。" },
                        "status": { "type": ["string", "null"], "enum": ["pending", "issued", "active", "rejected", "revoked", null] },
                        "online": { "type": ["string", "null"], "enum": ["online", "offline", null], "description": "综合在线状态：device-pki 会话 online 或最近 5 分钟内有 room-agent 快照。" },
                        "last_seen_at": { "type": ["string", "null"], "format": "date-time" },
                        "pending_token": { "type": "boolean", "description": "存在未使用且未过期的注册 token。" }
                    }
                },
                "DeviceInfo": {
                    "type": "object",
                    "required": ["serial", "device_slot", "status", "online", "last_seen_at", "downloaded", "client_info", "issued_at", "activated_at", "revoked_at", "room_id", "room_code", "room_name", "area_id", "area_name"],
                    "properties": {
                        "serial": { "type": "string" },
                        "device_slot": { "type": "string" },
                        "status": { "$ref": "#/components/schemas/DeviceStatus" },
                        "online": { "oneOf": [{ "$ref": "#/components/schemas/DeviceOnlineState" }, { "type": "null" }], "description": "综合在线状态：device-pki 会话 online 或最近 5 分钟内有 room-agent 快照。" },
                        "last_seen_at": { "type": ["string", "null"], "format": "date-time" },
                        "downloaded": { "type": "boolean" },
                        "client_info": { "type": ["object", "null"], "description": "设备注册时上报的客户端信息。" },
                        "issued_at": { "type": ["string", "null"], "format": "date-time" },
                        "activated_at": { "type": ["string", "null"], "format": "date-time" },
                        "revoked_at": { "type": ["string", "null"], "format": "date-time" },
                        "room_id": { "type": ["string", "null"], "format": "uuid", "description": "null = 未绑定房间。" },
                        "room_code": { "type": ["string", "null"] },
                        "room_name": { "type": ["string", "null"] },
                        "area_id": { "type": ["string", "null"], "format": "uuid" },
                        "area_name": { "type": ["string", "null"] }
                    }
                },
                "DevicePage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/DeviceInfo" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "DeviceTokenRequest": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": ["auto", "approval"], "default": "auto", "description": "auto = 自动注册；approval = 需审批。" },
                        "ttl_secs": { "type": "integer", "minimum": 60, "maximum": 86400, "default": 900 }
                    }
                },
                "DeviceTokenReply": {
                    "type": "object",
                    "required": ["token", "kind", "device_slot", "expires_at", "link"],
                    "properties": {
                        "token": { "type": "string", "description": "64 位 hex 明文 token，只返回一次。" },
                        "kind": { "type": "string", "enum": ["auto", "approval"] },
                        "device_slot": { "type": "string" },
                        "expires_at": { "type": "string", "format": "date-time" },
                        "link": { "type": "string", "format": "uri", "description": "设备注册链接：{PUBLIC_ENROLL_ENDPOINT}/enroll?token=…&slot=…" }
                    }
                },
                "RoomMachine": {
                    "type": "object",
                    "required": ["device_slot", "is_primary", "serial", "status", "online", "last_seen_at", "pending_token"],
                    "properties": {
                        "device_slot": { "type": "string" },
                        "is_primary": { "type": "boolean", "description": "true = 房间主设备（保留既有下发口径），false = 子机。" },
                        "label": { "type": ["string", "null"], "description": "子机可读标签。" },
                        "serial": { "type": ["string", "null"], "description": "null = 槽位已分配但设备尚未注册。" },
                        "status": { "type": ["string", "null"], "enum": ["pending", "issued", "active", "rejected", "revoked", null] },
                        "online": { "type": ["string", "null"], "enum": ["online", "offline", null], "description": "综合在线状态：device-pki 会话 online 或最近 5 分钟内有机器快照。" },
                        "last_seen_at": { "type": ["string", "null"], "format": "date-time" },
                        "pending_token": { "type": "boolean", "description": "存在未使用且未过期的注册 token。" },
                        "created_at": { "type": ["string", "null"], "format": "date-time", "description": "绑定到房间的时间。" }
                    }
                },
                "MachineTokenRequest": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string", "enum": ["auto", "approval"], "default": "auto", "description": "auto = 自动注册；approval = 需审批。" },
                        "ttl_secs": { "type": "integer", "minimum": 60, "maximum": 86400, "default": 900 },
                        "label": { "type": "string", "maxLength": 64, "description": "可读标签（如“左侧投影机”）。" }
                    }
                },
                "CreateRoomRequest": {
                    "type": "object",
                    "required": ["area_id", "code", "name"],
                    "properties": {
                        "area_id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "capacity": { "type": "integer", "minimum": 0, "maximum": 100000 },
                        "device_slot": { "type": ["string", "null"] },
                        "webhook_url": { "type": ["string", "null"], "format": "uri" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": "object" }
                    }
                },
                "UpdateRoomRequest": {
                    "type": "object",
                    "properties": {
                        "area_id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "capacity": { "type": "integer" },
                        "device_slot": { "type": ["string", "null"], "description": "缺失 = 不修改；null = 清空。" },
                        "webhook_url": { "type": ["string", "null"], "description": "缺失 = 不修改；null = 清空。" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": ["object", "null"] }
                    }
                },
                "Schedule": {
                    "type": "object",
                    "required": ["id", "room_id", "room_name", "name", "start_date", "end_date", "timezone", "is_active", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "room_id": { "type": "string", "format": "uuid" },
                        "room_name": { "type": "string" },
                        "name": { "type": "string" },
                        "start_date": { "type": "string", "format": "date" },
                        "end_date": { "type": "string", "format": "date" },
                        "timezone": { "type": "string", "readOnly": true, "example": "Asia/Shanghai", "description": "只读：继承自房间所属区域；周模板 TIME 按此时区解释。" },
                        "is_active": { "type": "boolean" },
                        "created_by": { "type": ["string", "null"], "format": "uuid" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "SchedulePage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/Schedule" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "BoardDevice": {
                    "type": "object",
                    "required": ["serial", "status", "online"],
                    "properties": {
                        "serial": { "type": "string", "description": "设备序列号（device_enrollment.serial）。" },
                        "status": { "type": ["string", "null"], "enum": ["pending", "issued", "active", "rejected", "revoked", null], "description": "设备注册状态。" },
                        "online": { "type": ["boolean", "null"], "description": "设备在线状态；取不到为 null。" }
                    }
                },
                "BoardArea": {
                    "type": "object",
                    "required": ["id", "code", "name", "timezone"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "timezone": { "type": "string", "example": "Asia/Shanghai", "description": "IANA 时区（继承自区域）。" }
                    }
                },
                "BoardRoom": {
                    "type": "object",
                    "required": ["id", "area_id", "area_name", "code", "name", "capacity", "device_slot", "is_active", "timezone", "device"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "area_id": { "type": "string", "format": "uuid" },
                        "area_name": { "type": "string" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "capacity": { "type": "integer", "minimum": 0 },
                        "device_slot": { "type": ["string", "null"] },
                        "is_active": { "type": "boolean", "description": "工作台只返回启用中的房间，恒为 true。" },
                        "timezone": { "type": "string", "example": "Asia/Shanghai" },
                        "device": { "oneOf": [{ "$ref": "#/components/schemas/BoardDevice" }, { "type": "null" }], "description": "null = 房间未绑定槽位或无 enrollment。" }
                    }
                },
                "BoardSchedule": {
                    "type": "object",
                    "required": ["id", "room_id", "name", "start_date", "end_date", "is_active", "timezone"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "room_id": { "type": "string", "format": "uuid" },
                        "name": { "type": "string" },
                        "start_date": { "type": "string", "format": "date" },
                        "end_date": { "type": "string", "format": "date" },
                        "is_active": { "type": "boolean", "description": "工作台只返回启用中的课表，恒为 true。" },
                        "timezone": { "type": "string", "example": "Asia/Shanghai" }
                    }
                },
                "BoardWeeklySlot": {
                    "type": "object",
                    "required": ["id", "schedule_id", "day_of_week", "start_time", "end_time", "course_name", "teacher", "room_id", "note"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "schedule_id": { "type": "string", "format": "uuid" },
                        "day_of_week": { "type": "integer", "minimum": 1, "maximum": 7, "description": "ISO 星期：周一=1 .. 周日=7。" },
                        "start_time": { "type": "string", "example": "08:00" },
                        "end_time": { "type": "string", "example": "09:40" },
                        "course_name": { "type": "string" },
                        "teacher": { "type": "string" },
                        "room_id": { "type": ["string", "null"], "format": "uuid", "description": "调课到其他房间；null = 课表所属房间。" },
                        "note": { "type": "string" }
                    }
                },
                "BoardExceptionSlot": {
                    "type": "object",
                    "required": ["id", "start_time", "end_time", "course_name", "teacher", "room_id", "note"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "start_time": { "type": "string", "example": "08:00" },
                        "end_time": { "type": "string", "example": "09:40" },
                        "course_name": { "type": "string" },
                        "teacher": { "type": "string" },
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "note": { "type": "string" }
                    }
                },
                "BoardException": {
                    "type": "object",
                    "required": ["id", "schedule_id", "exception_date", "kind", "note", "slots"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "schedule_id": { "type": "string", "format": "uuid" },
                        "exception_date": { "type": "string", "format": "date" },
                        "kind": { "type": "string", "enum": ["cancel", "replace"] },
                        "note": { "type": "string" },
                        "slots": { "type": "array", "items": { "$ref": "#/components/schemas/BoardExceptionSlot" }, "description": "按 start_time 排序。" }
                    }
                },
                "ScheduleBoard": {
                    "type": "object",
                    "required": ["from", "to", "areas", "rooms", "schedules", "weekly_slots", "exceptions"],
                    "properties": {
                        "from": { "type": "string", "format": "date" },
                        "to": { "type": "string", "format": "date" },
                        "areas": { "type": "array", "items": { "$ref": "#/components/schemas/BoardArea" }, "description": "全部区域（含停用），按 name 排序。" },
                        "rooms": { "type": "array", "items": { "$ref": "#/components/schemas/BoardRoom" }, "description": "启用中的房间，按 area.name + room.code 排序。" },
                        "schedules": { "type": "array", "items": { "$ref": "#/components/schemas/BoardSchedule" }, "description": "属于上述房间的启用课表（不裁剪日期）。" },
                        "weekly_slots": { "type": "array", "items": { "$ref": "#/components/schemas/BoardWeeklySlot" }, "description": "上述课表的全部周模板，按 day_of_week + start_time 排序。" },
                        "exceptions": { "type": "array", "items": { "$ref": "#/components/schemas/BoardException" }, "description": "上述课表在 [from, to] 内的单日覆盖。" }
                    }
                },
                "EnsureSchedulesRequest": {
                    "type": "object",
                    "required": ["room_ids", "start_date", "end_date"],
                    "properties": {
                        "room_ids": { "type": "array", "items": { "type": "string", "format": "uuid" }, "minItems": 1, "maxItems": 100, "description": "去重后 1..=100 个房间；房间必须存在且启用。" },
                        "start_date": { "type": "string", "format": "date" },
                        "end_date": { "type": "string", "format": "date", "description": "必须 >= start_date。" },
                        "name": { "type": "string", "maxLength": 200, "default": "默认课表" }
                    }
                },
                "EnsureSchedulesResponse": {
                    "type": "object",
                    "required": ["created", "existing"],
                    "properties": {
                        "created": { "type": "array", "items": { "$ref": "#/components/schemas/Schedule" }, "description": "本次新建的课表（沿用 Schedule 结构）。" },
                        "existing": { "type": "array", "items": { "$ref": "#/components/schemas/Schedule" }, "description": "已存在且与目标日期区间有交集的启用课表（每房间取 start_date 最早一份）。" }
                    }
                },
                "CreateScheduleRequest": {
                    "type": "object",
                    "required": ["room_id", "name", "start_date", "end_date"],
                    "properties": {
                        "room_id": { "type": "string", "format": "uuid" },
                        "name": { "type": "string", "maxLength": 200 },
                        "start_date": { "type": "string", "format": "date" },
                        "end_date": { "type": "string", "format": "date" },
                        "is_active": { "type": "boolean" }
                    }
                },
                "UpdateScheduleRequest": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "start_date": { "type": "string", "format": "date" },
                        "end_date": { "type": "string", "format": "date" },
                        "is_active": { "type": "boolean" }
                    }
                },
                "NewWeeklySlot": {
                    "type": "object",
                    "required": ["day_of_week", "start_time", "end_time", "course_name"],
                    "properties": {
                        "day_of_week": { "type": "integer", "minimum": 1, "maximum": 7, "description": "ISO 星期：周一=1 .. 周日=7。" },
                        "start_time": { "type": "string", "example": "08:00" },
                        "end_time": { "type": "string", "example": "09:40" },
                        "course_name": { "type": "string" },
                        "teacher": { "type": "string" },
                        "room_id": { "type": ["string", "null"], "format": "uuid", "description": "调课到其他房间；空 = 课表所属房间。" },
                        "note": { "type": "string" }
                    }
                },
                "WeeklySlot": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "schedule_id": { "type": "string", "format": "uuid" },
                        "day_of_week": { "type": "integer" },
                        "start_time": { "type": "string" },
                        "end_time": { "type": "string" },
                        "course_name": { "type": "string" },
                        "teacher": { "type": "string" },
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "room_name": { "type": ["string", "null"] },
                        "note": { "type": "string" },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "ReplaceWeeklySlotsRequest": {
                    "type": "object",
                    "required": ["slots"],
                    "properties": {
                        "slots": { "type": "array", "items": { "$ref": "#/components/schemas/NewWeeklySlot" }, "maxItems": 200 }
                    }
                },
                "NewExceptionSlot": {
                    "type": "object",
                    "required": ["start_time", "end_time", "course_name"],
                    "properties": {
                        "start_time": { "type": "string" },
                        "end_time": { "type": "string" },
                        "course_name": { "type": "string" },
                        "teacher": { "type": "string" },
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "note": { "type": "string" }
                    }
                },
                "ScheduleExceptionSlot": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "exception_id": { "type": "string", "format": "uuid" },
                        "start_time": { "type": "string" },
                        "end_time": { "type": "string" },
                        "course_name": { "type": "string" },
                        "teacher": { "type": "string" },
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "room_name": { "type": ["string", "null"] },
                        "note": { "type": "string" }
                    }
                },
                "ScheduleException": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "schedule_id": { "type": "string", "format": "uuid" },
                        "exception_date": { "type": "string", "format": "date" },
                        "kind": { "type": "string", "enum": ["cancel", "replace"] },
                        "note": { "type": "string" },
                        "slots": { "type": "array", "items": { "$ref": "#/components/schemas/ScheduleExceptionSlot" } },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "UpsertExceptionRequest": {
                    "type": "object",
                    "required": ["kind"],
                    "properties": {
                        "kind": { "type": "string", "enum": ["cancel", "replace"] },
                        "note": { "type": "string" },
                        "slots": { "type": "array", "items": { "$ref": "#/components/schemas/NewExceptionSlot" }, "maxItems": 100 }
                    }
                },
                "Occurrence": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "schedule_id": { "type": "string", "format": "uuid" },
                        "room_id": { "type": "string", "format": "uuid" },
                        "occurrence_date": { "type": "string", "format": "date" },
                        "start_at": { "type": "string", "format": "date-time", "description": "UTC 时间点。" },
                        "end_at": { "type": "string", "format": "date-time", "description": "UTC 时间点。" },
                        "timezone": { "type": "string", "example": "Asia/Shanghai", "description": "只读：来自课表→房间→区域。" },
                        "start_local": { "type": "string", "example": "2026-11-02T08:00:00+08:00", "description": "按 timezone 格式化的本地开始时间。" },
                        "end_local": { "type": "string", "example": "2026-11-02T09:40:00+08:00", "description": "按 timezone 格式化的本地结束时间。" },
                        "course_name": { "type": "string" },
                        "teacher": { "type": "string" },
                        "source": { "type": "string", "enum": ["weekly", "custom"] },
                        "source_slot_id": { "type": "string", "format": "uuid" },
                        "note": { "type": "string" }
                    }
                },
                "EventRule": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "room_id": { "type": "string", "format": "uuid" },
                        "trigger_kind": { "type": "string", "enum": ["occurrence_start", "occurrence_end", "occurrence_start_before", "daily_time", "room_occupied", "room_idle"] },
                        "offset_minutes": { "type": "integer", "minimum": 0, "maximum": 1440 },
                        "at_time": { "type": ["string", "null"], "example": "07:30", "description": "仅 daily_time 使用且必填。" },
                        "enabled": { "type": "boolean" },
                        "config": { "type": "object", "description": "targets / webhook_url / downlink_kind / payload。" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "NewEventRule": {
                    "type": "object",
                    "required": ["trigger_kind"],
                    "properties": {
                        "trigger_kind": { "type": "string", "enum": ["occurrence_start", "occurrence_end", "occurrence_start_before", "daily_time", "room_occupied", "room_idle"] },
                        "offset_minutes": { "type": "integer", "minimum": 0, "maximum": 1440 },
                        "at_time": { "type": ["string", "null"] },
                        "enabled": { "type": "boolean", "default": true },
                        "config": { "type": "object" }
                    }
                },
                "ReplaceEventRulesRequest": {
                    "type": "object",
                    "required": ["rules"],
                    "properties": {
                        "rules": { "type": "array", "items": { "$ref": "#/components/schemas/NewEventRule" }, "maxItems": 50 }
                    }
                },
                "ScheduleEvent": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "event_key": { "type": "string" },
                        "room_id": { "type": "string", "format": "uuid" },
                        "schedule_id": { "type": ["string", "null"], "format": "uuid" },
                        "occurrence_id": { "type": ["string", "null"], "format": "uuid" },
                        "rule_id": { "type": ["string", "null"], "format": "uuid" },
                        "event_kind": { "type": "string" },
                        "title": { "type": "string" },
                        "payload": { "type": "object" },
                        "timezone": { "type": "string", "example": "Asia/Shanghai", "description": "事件生成时所属区域时区快照。" },
                        "triggered_at": { "type": "string", "format": "date-time", "description": "UTC 时间点。" },
                        "triggered_at_local": { "type": "string", "example": "2026-11-02T08:00:00+08:00", "description": "按 timezone 格式化的本地触发时间。" },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "ScheduleEventPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/ScheduleEvent" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "ScheduleEventDelivery": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "integer" },
                        "event_id": { "type": "string", "format": "uuid" },
                        "target": { "type": "string", "enum": ["pubsub", "downlink", "webhook"] },
                        "status": { "type": "string", "enum": ["pending", "delivered", "failed", "skipped"] },
                        "attempts": { "type": "integer" },
                        "next_attempt_at": { "type": "string", "format": "date-time" },
                        "last_error": { "type": ["string", "null"] },
                        "delivered_at": { "type": ["string", "null"], "format": "date-time" },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "ScheduleEventDetail": {
                    "type": "object",
                    "properties": {
                        "event": { "$ref": "#/components/schemas/ScheduleEvent" },
                        "deliveries": { "type": "array", "items": { "$ref": "#/components/schemas/ScheduleEventDelivery" } }
                    }
                },
                "RebuildResult": {
                    "type": "object",
                    "properties": { "occurrences": { "type": "integer" } }
                },
                "ReplayResult": {
                    "type": "object",
                    "properties": { "reset": { "type": "integer" } }
                },
                "ImageKind": { "type": "string", "enum": ["docker", "oci", "lxc_rootfs", "vm_disk"] },
                "ImageFormat": { "type": "string", "enum": ["docker_archive", "oci_layout", "tar", "qcow2", "raw"] },
                "Compression": { "type": "string", "enum": ["none", "zstd", "gzip"], "description": "P0 只接受 none。" },
                "ArtifactStatus": { "type": "string", "enum": ["uploaded", "manifest_ready", "verifying", "verified", "published", "deprecated", "revoked"] },
                "ImageChannel": { "type": "string", "enum": ["dev", "test", "prod"] },
                "TransferStatus": { "type": "string", "enum": ["pending", "running", "completed", "failed", "expired"] },
                "CollectionStatus": { "type": "string", "enum": ["pending", "uploaded", "verified", "rejected"] },
                "StorageObject": {
                    "type": "object",
                    "required": ["id", "bucket", "object_key", "sha256", "size", "content_type", "created_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "bucket": { "type": "string" },
                        "object_key": { "type": "string", "example": "objects/ab12…" },
                        "sha256": { "type": "string", "description": "64 位小写 hex；内容寻址键。" },
                        "size": { "type": "integer", "format": "int64" },
                        "content_type": { "type": "string" },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "StorageObjectRequest": {
                    "type": "object",
                    "required": ["sha256", "size"],
                    "properties": {
                        "sha256": { "type": "string", "description": "对象内容的 SHA-256（小写 hex）。" },
                        "size": { "type": "integer", "format": "int64", "minimum": 1, "maximum": 67108864, "description": "P0 上限 64 MiB（服务端登记需读回内容）。" },
                        "content_type": { "type": ["string", "null"], "default": "application/octet-stream" }
                    }
                },
                "PresignObjectReply": {
                    "type": "object",
                    "required": ["bucket", "object_key", "upload_url", "expires_at"],
                    "properties": {
                        "bucket": { "type": "string" },
                        "object_key": { "type": "string", "example": "objects/ab12…" },
                        "upload_url": { "type": "string", "format": "uri", "description": "短 TTL 预签名 PUT 地址。" },
                        "expires_at": { "type": "string", "format": "date-time" }
                    }
                },
                "StorageObjectReply": {
                    "type": "object",
                    "required": ["object", "download_url", "expires_at"],
                    "properties": {
                        "object": { "$ref": "#/components/schemas/StorageObject" },
                        "download_url": { "type": "string", "format": "uri", "description": "短 TTL 预签名 GET 地址。" },
                        "expires_at": { "type": "string", "format": "date-time" }
                    }
                },
                "Transfer": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "direction": { "type": "string", "enum": ["download", "upload"] },
                        "room_id": { "type": "string", "format": "uuid" },
                        "image_id": { "type": ["string", "null"], "format": "uuid" },
                        "manifest_digest": { "type": ["string", "null"] },
                        "status": { "$ref": "#/components/schemas/TransferStatus" },
                        "progress": { "type": "integer", "minimum": 0, "maximum": 100 },
                        "error": { "type": ["string", "null"] },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "TransferPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/Transfer" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "Collection": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "area_id": { "type": ["string", "null"], "format": "uuid" },
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "kind": { "type": "string" },
                        "object_key": { "type": "string" },
                        "sha256": { "type": "string" },
                        "size": { "type": "integer", "format": "int64" },
                        "status": { "$ref": "#/components/schemas/CollectionStatus" },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "CollectionPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/Collection" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "ImageArtifact": {
                    "type": "object",
                    "required": ["id", "name", "version", "kind", "format", "compression", "platform", "total_size", "status", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "name": { "type": "string" },
                        "version": { "type": "string" },
                        "kind": { "$ref": "#/components/schemas/ImageKind" },
                        "format": { "$ref": "#/components/schemas/ImageFormat" },
                        "compression": { "$ref": "#/components/schemas/Compression" },
                        "platform": { "type": "string", "example": "linux/amd64" },
                        "manifest_digest": { "type": ["string", "null"], "description": "manifest 的 sha256（小写 hex）；提交 manifest 后写入。" },
                        "total_size": { "type": "integer", "format": "int64" },
                        "status": { "$ref": "#/components/schemas/ArtifactStatus" },
                        "created_by": { "type": ["string", "null"], "format": "uuid" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "ImagePage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/ImageArtifact" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "ImageChunk": {
                    "type": "object",
                    "properties": {
                        "manifest_digest": { "type": "string" },
                        "index": { "type": "integer", "minimum": 0 },
                        "object_key": { "type": "string" },
                        "sha256": { "type": "string" },
                        "size": { "type": "integer", "format": "int64" },
                        "offset": { "type": "integer", "format": "int64", "description": "逻辑拼接偏移；从 0 连续。" }
                    }
                },
                "ImageVerification": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "artifact_id": { "type": "string", "format": "uuid" },
                        "ok": { "type": "boolean" },
                        "report": {
                            "type": "object",
                            "properties": {
                                "manifest_digest": { "type": "string" },
                                "checks": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "ok": { "type": "boolean" }, "detail": { "type": "string" } } } },
                                "errors": { "type": "array", "items": { "type": "string" } }
                            }
                        },
                        "verified_by": { "type": "string", "format": "uuid" },
                        "verified_at": { "type": "string", "format": "date-time" }
                    }
                },
                "ImageChannelRow": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "artifact_id": { "type": "string", "format": "uuid" },
                        "channel": { "$ref": "#/components/schemas/ImageChannel" },
                        "is_current": { "type": "boolean" },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "ImageAssignment": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "image_id": { "type": "string", "format": "uuid" },
                        "area_id": { "type": ["string", "null"], "format": "uuid" },
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "channel": { "$ref": "#/components/schemas/ImageChannel" },
                        "rollout_state": { "type": "string" },
                        "assigned_by": { "type": "string", "format": "uuid" },
                        "assigned_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "ImageDetail": {
                    "allOf": [
                        { "$ref": "#/components/schemas/ImageArtifact" },
                        {
                            "type": "object",
                            "properties": {
                                "chunks": { "type": "array", "items": { "$ref": "#/components/schemas/ImageChunk" } },
                                "latest_verification": { "oneOf": [{ "$ref": "#/components/schemas/ImageVerification" }, { "type": "null" }] },
                                "channels": { "type": "array", "items": { "$ref": "#/components/schemas/ImageChannelRow" } }
                            }
                        }
                    ]
                },
                "CreateImageRequest": {
                    "type": "object",
                    "required": ["name", "version", "kind", "format", "platform"],
                    "properties": {
                        "name": { "type": "string", "description": "1-64 位小写字母数字 - _ ." },
                        "version": { "type": "string", "description": "1-64 位字母数字 . - _ +" },
                        "kind": { "$ref": "#/components/schemas/ImageKind" },
                        "format": { "$ref": "#/components/schemas/ImageFormat" },
                        "compression": { "$ref": "#/components/schemas/Compression", "default": "none" },
                        "platform": { "type": "string", "example": "linux/amd64" }
                    }
                },
                "ManifestPart": {
                    "type": "object",
                    "required": ["index", "sha256", "size", "offset"],
                    "properties": {
                        "index": { "type": "integer", "minimum": 0, "description": "从 0 连续递增。" },
                        "sha256": { "type": "string", "description": "必须指向已登记的 `objects/{sha256}`。" },
                        "size": { "type": "integer", "format": "int64", "minimum": 1 },
                        "offset": { "type": "integer", "format": "int64", "description": "逻辑偏移；从 0 连续。" }
                    }
                },
                "ManifestRuntimeInput": {
                    "type": "object",
                    "properties": {
                        "docker": { "type": "object", "properties": { "image_ref": { "type": "string" } } },
                        "lxc": { "type": "object", "properties": { "distro": { "type": "string" }, "release": { "type": "string" } } },
                        "vm": { "type": "object", "properties": { "os": { "type": "string" }, "disk_size": { "type": "integer", "format": "int64" } } }
                    }
                },
                "SubmitManifestRequest": {
                    "type": "object",
                    "required": ["total_size", "parts"],
                    "properties": {
                        "total_size": { "type": "integer", "format": "int64", "minimum": 1, "description": "必须等于分片 size 之和。" },
                        "parts": { "type": "array", "minItems": 1, "items": { "$ref": "#/components/schemas/ManifestPart" } },
                        "runtime": { "$ref": "#/components/schemas/ManifestRuntimeInput" },
                        "annotations": { "type": "object" }
                    }
                },
                "ManifestRuntime": {
                    "type": "object",
                    "properties": {
                        "docker_image_ref": { "type": ["string", "null"] },
                        "lxc_distro": { "type": ["string", "null"] },
                        "lxc_release": { "type": ["string", "null"] },
                        "vm_os": { "type": ["string", "null"] },
                        "vm_disk_size": { "type": ["integer", "null"], "format": "int64" }
                    }
                },
                "ManifestChunk": {
                    "type": "object",
                    "properties": {
                        "index": { "type": "integer" },
                        "object_key": { "type": "string" },
                        "sha256": { "type": "string" },
                        "size": { "type": "integer", "format": "int64" },
                        "offset": { "type": "integer", "format": "int64" }
                    }
                },
                "Manifest": {
                    "type": "object",
                    "properties": {
                        "schema": { "type": "integer", "const": 1 },
                        "kind": { "$ref": "#/components/schemas/ImageKind" },
                        "name": { "type": "string" },
                        "version": { "type": "string" },
                        "platform": { "type": "string" },
                        "format": { "$ref": "#/components/schemas/ImageFormat" },
                        "compression": { "$ref": "#/components/schemas/Compression" },
                        "total_size": { "type": "integer", "format": "int64" },
                        "chunks": { "type": "array", "items": { "$ref": "#/components/schemas/ManifestChunk" } },
                        "runtime": { "$ref": "#/components/schemas/ManifestRuntime" },
                        "annotations": { "type": "object" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "created_by": { "type": ["string", "null"], "format": "uuid" }
                    }
                },
                "SubmitManifestReply": {
                    "type": "object",
                    "required": ["artifact", "manifest_digest", "manifest"],
                    "properties": {
                        "artifact": { "$ref": "#/components/schemas/ImageArtifact" },
                        "manifest_digest": { "type": "string", "description": "manifest 原始字节的 sha256（小写 hex）。" },
                        "manifest": { "$ref": "#/components/schemas/Manifest" }
                    }
                },
                "VerifyReply": {
                    "type": "object",
                    "required": ["artifact", "verification"],
                    "properties": {
                        "artifact": { "$ref": "#/components/schemas/ImageArtifact" },
                        "verification": { "$ref": "#/components/schemas/ImageVerification" }
                    }
                },
                "ManifestReply": {
                    "type": "object",
                    "required": ["artifact_id", "manifest_digest", "manifest", "download_url", "expires_at"],
                    "properties": {
                        "artifact_id": { "type": "string", "format": "uuid" },
                        "manifest_digest": { "type": "string" },
                        "manifest": { "$ref": "#/components/schemas/Manifest" },
                        "download_url": { "type": "string", "format": "uri" },
                        "expires_at": { "type": "string", "format": "date-time" }
                    }
                },
                "PublishImageRequest": {
                    "type": "object",
                    "required": ["channel"],
                    "properties": { "channel": { "$ref": "#/components/schemas/ImageChannel" } }
                },
                "AssignImageRequest": {
                    "type": "object",
                    "properties": { "channel": { "$ref": "#/components/schemas/ImageChannel", "default": "prod" } }
                },
                "AgentConfigScopeType": {
                    "type": "string",
                    "enum": ["global", "area", "room"],
                    "description": "作用域：global（scope_id 必须为空）/ area / room（必须提供 scope_id）。"
                },
                "AgentPolicy": {
                    "type": "object",
                    "description": "合并后的完整策略（默认值：lxc_auto_start=false、poll_interval_secs=2、download_concurrency=4、cache_max_bytes=10737418240）。",
                    "required": ["lxc_auto_start", "poll_interval_secs", "download_concurrency", "cache_max_bytes"],
                    "properties": {
                        "lxc_auto_start": { "type": "boolean" },
                        "poll_interval_secs": { "type": "integer", "minimum": 1, "maximum": 60 },
                        "download_concurrency": { "type": "integer", "minimum": 1, "maximum": 32 },
                        "cache_max_bytes": { "type": "integer", "minimum": 0, "maximum": 1099511627776i64 }
                    }
                },
                "AgentPolicyPatch": {
                    "type": "object",
                    "description": "局部策略；只覆盖出现的字段，范围与 AgentPolicy 一致。",
                    "properties": {
                        "lxc_auto_start": { "type": "boolean" },
                        "poll_interval_secs": { "type": "integer", "minimum": 1, "maximum": 60 },
                        "download_concurrency": { "type": "integer", "minimum": 1, "maximum": 32 },
                        "cache_max_bytes": { "type": "integer", "minimum": 0, "maximum": 1099511627776i64 }
                    }
                },
                "AgentConfig": {
                    "type": "object",
                    "required": ["id", "scope_type", "scope_id", "policy", "revision", "updated_by", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "scope_type": { "$ref": "#/components/schemas/AgentConfigScopeType" },
                        "scope_id": { "type": ["string", "null"], "format": "uuid" },
                        "policy": { "$ref": "#/components/schemas/AgentPolicyPatch" },
                        "revision": { "type": "integer", "description": "每次 upsert 递增；新行从 1 开始。" },
                        "updated_by": { "type": ["string", "null"], "format": "uuid" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "AgentConfigList": {
                    "type": "object",
                    "required": ["items", "total"],
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/AgentConfig" } },
                        "total": { "type": "integer" }
                    }
                },
                "UpdateAgentConfigRequest": {
                    "type": "object",
                    "required": ["scope_type", "policy"],
                    "properties": {
                        "scope_type": { "$ref": "#/components/schemas/AgentConfigScopeType" },
                        "scope_id": { "type": ["string", "null"], "format": "uuid", "description": "global 必须缺省/为 null；area/room 必填。" },
                        "policy": { "$ref": "#/components/schemas/AgentPolicyPatch" }
                    }
                },
                "UpdateAgentConfigReply": {
                    "type": "object",
                    "required": ["config", "pushed_rooms"],
                    "properties": {
                        "config": { "$ref": "#/components/schemas/AgentConfig" },
                        "pushed_rooms": { "type": "integer", "description": "成功写入 config.update 下行的房间数。" }
                    }
                },
                "EffectiveAgentPolicyReply": {
                    "type": "object",
                    "required": ["room_id", "policy", "revision"],
                    "properties": {
                        "room_id": { "type": "string", "format": "uuid" },
                        "policy": { "$ref": "#/components/schemas/AgentPolicy" },
                        "revision": { "type": "integer", "description": "已应用行中的最大 revision；无配置为 0。" }
                    }
                },
                "PresenceEvent": {
                    "type": "object",
                    "required": ["id", "serial", "device_slot", "previous_state", "current_state", "occurred_at", "payload"],
                    "properties": {
                        "id": { "type": "integer", "format": "int64" },
                        "serial": { "type": "string" },
                        "device_slot": { "type": "string" },
                        "room_id": { "type": ["string", "null"], "format": "uuid", "description": "null = 事件发生时设备未绑定房间。" },
                        "area_id": { "type": ["string", "null"], "format": "uuid" },
                        "previous_state": { "type": "string", "enum": ["online", "offline"] },
                        "current_state": { "type": "string", "enum": ["online", "offline"] },
                        "occurred_at": { "type": "string", "format": "date-time" },
                        "payload": { "type": "object", "description": "worker 组装的投递/SSE 载荷（含房间/区域快照）。" }
                    }
                },
                "PresenceEventList": {
                    "type": "object",
                    "required": ["items", "total"],
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/PresenceEvent" } },
                        "total": { "type": "integer", "description": "本次返回条数（不是全表总数）。" }
                    }
                },
                "PresenceDelivery": {
                    "type": "object",
                    "required": ["id", "event_id", "target_url", "status", "attempts", "next_attempt_at", "last_attempt_at", "last_error"],
                    "properties": {
                        "id": { "type": "integer", "format": "int64" },
                        "event_id": { "type": "integer", "format": "int64" },
                        "target_url": { "type": "string", "format": "uri" },
                        "status": { "type": "string", "enum": ["pending", "delivered", "failed"] },
                        "attempts": { "type": "integer" },
                        "next_attempt_at": { "type": "string", "format": "date-time" },
                        "last_attempt_at": { "type": ["string", "null"], "format": "date-time" },
                        "last_error": { "type": ["string", "null"] }
                    }
                },
                "PresenceDeliveryList": {
                    "type": "object",
                    "required": ["items", "total"],
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/PresenceDelivery" } },
                        "total": { "type": "integer", "description": "本次返回条数。" }
                    }
                },
                "MachineIssue": {
                    "type": "object",
                    "required": ["severity", "code", "message"],
                    "properties": {
                        "severity": { "type": "string", "enum": ["info", "warning", "critical"] },
                        "code": { "type": "string", "description": "稳定的问题码，例如 disk.usage / thermal.temperature。" },
                        "message": { "type": "string" },
                        "detail": { "type": ["string", "null"] }
                    }
                },
                "MachineHealth": {
                    "type": "object",
                    "required": ["status"],
                    "properties": {
                        "status": { "type": "string", "enum": ["ok", "warning", "critical"] },
                        "score": { "type": ["integer", "null"], "minimum": 0, "maximum": 100 },
                        "issues": { "type": "array", "items": { "$ref": "#/components/schemas/MachineIssue" } }
                    }
                },
                "MachineMetrics": {
                    "type": "object",
                    "properties": {
                        "cpu_usage_percent": { "type": ["number", "null"], "minimum": 0, "maximum": 100 },
                        "memory_usage_percent": { "type": ["number", "null"], "minimum": 0, "maximum": 100 },
                        "disk_usage_percent": { "type": ["number", "null"], "minimum": 0, "maximum": 100 },
                        "uptime_secs": { "type": ["integer", "null"], "minimum": 0 }
                    }
                },
                "MachineReportRequest": {
                    "type": "object",
                    "required": ["device_slot", "agent_version", "hostname", "health"],
                    "properties": {
                        "schema": { "type": "integer", "const": 1, "default": 1 },
                        "device_slot": { "type": "string", "maxLength": 128 },
                        "agent_version": { "type": "string", "maxLength": 64 },
                        "collected_at": { "type": "string", "format": "date-time", "description": "缺省为服务端当前时间；未来超过 5 分钟会被拒绝。" },
                        "hostname": { "type": "string", "maxLength": 255 },
                        "os_name": { "type": ["string", "null"] },
                        "kernel_version": { "type": ["string", "null"] },
                        "health": { "$ref": "#/components/schemas/MachineHealth" },
                        "metrics": { "$ref": "#/components/schemas/MachineMetrics" },
                        "sections": { "type": "object", "description": "各采集域原始 JSON：platform_health / diagnose / security / containers / vms / agent。" }
                    }
                },
                "MachineReportReply": {
                    "type": "object",
                    "required": ["snapshot_id", "serial", "device_slot", "received_at"],
                    "properties": {
                        "snapshot_id": { "type": "integer", "format": "int64" },
                        "serial": { "type": "string" },
                        "device_slot": { "type": "string" },
                        "received_at": { "type": "string", "format": "date-time" }
                    }
                },
                "MachineReportSummary": {
                    "type": "object",
                    "required": ["collected_at", "received_at", "agent_version", "hostname", "health_status"],
                    "properties": {
                        "collected_at": { "type": "string", "format": "date-time" },
                        "received_at": { "type": "string", "format": "date-time" },
                        "agent_version": { "type": "string" },
                        "hostname": { "type": "string" },
                        "os_name": { "type": ["string", "null"] },
                        "kernel_version": { "type": ["string", "null"] },
                        "health_status": { "type": "string", "enum": ["ok", "warning", "critical", "unknown"] },
                        "health_score": { "type": ["integer", "null"] },
                        "cpu_usage_percent": { "type": ["number", "null"] },
                        "memory_usage_percent": { "type": ["number", "null"] },
                        "disk_usage_percent": { "type": ["number", "null"] },
                        "uptime_secs": { "type": ["integer", "null"] }
                    }
                },
                "MachineSummary": {
                    "type": "object",
                    "required": ["serial", "device_slot", "status", "online", "session_online", "last_seen_at", "room_id", "room_code", "room_name", "area_id", "area_code", "area_name", "report", "report_stale"],
                    "properties": {
                        "serial": { "type": "string" },
                        "device_slot": { "type": "string" },
                        "status": { "type": "string", "enum": ["pending", "issued", "active", "rejected", "revoked"] },
                        "online": { "type": ["string", "null"], "enum": ["online", "offline", null], "description": "综合在线状态：device-pki 会话 online 或最近 5 分钟内有 room-agent 快照。" },
                        "session_online": { "type": ["string", "null"], "enum": ["online", "offline", null], "description": "device-pki 的 mTLS 会话原始状态，仅用于排障展示。" },
                        "last_seen_at": { "type": ["string", "null"], "format": "date-time", "description": "最近一次 device-pki 心跳或 room-agent 快照接收时间。" },
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "room_code": { "type": ["string", "null"] },
                        "room_name": { "type": ["string", "null"] },
                        "area_id": { "type": ["string", "null"], "format": "uuid" },
                        "area_code": { "type": ["string", "null"] },
                        "area_name": { "type": ["string", "null"] },
                        "report": { "oneOf": [{ "$ref": "#/components/schemas/MachineReportSummary" }, { "type": "null" }], "description": "null = 从未上报。" },
                        "report_stale": { "type": "boolean", "description": "最新快照超过 5 分钟未更新。" }
                    }
                },
                "MachinePage": {
                    "type": "object",
                    "required": ["items", "total", "page", "page_size"],
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/MachineSummary" } },
                        "total": { "type": "integer" },
                        "page": { "type": "integer" },
                        "page_size": { "type": "integer" }
                    }
                },
                "MachineSnapshotPoint": {
                    "type": "object",
                    "required": ["id", "serial", "device_slot", "collected_at", "received_at", "agent_version", "hostname", "health_status"],
                    "properties": {
                        "id": { "type": "integer", "format": "int64" },
                        "serial": { "type": "string" },
                        "device_slot": { "type": "string" },
                        "collected_at": { "type": "string", "format": "date-time" },
                        "received_at": { "type": "string", "format": "date-time" },
                        "agent_version": { "type": "string" },
                        "hostname": { "type": "string" },
                        "os_name": { "type": ["string", "null"] },
                        "kernel_version": { "type": ["string", "null"] },
                        "health_status": { "type": "string", "enum": ["ok", "warning", "critical"] },
                        "health_score": { "type": ["integer", "null"] },
                        "cpu_usage_percent": { "type": ["number", "null"] },
                        "memory_usage_percent": { "type": ["number", "null"] },
                        "disk_usage_percent": { "type": ["number", "null"] },
                        "uptime_secs": { "type": ["integer", "null"] }
                    }
                },
                "MachineSnapshot": {
                    "allOf": [
                        { "$ref": "#/components/schemas/MachineSnapshotPoint" },
                        {
                            "type": "object",
                            "required": ["payload"],
                            "properties": {
                                "payload": { "type": "object", "description": "room-agent 上报的完整 JSON（sections 原样保存）。" }
                            }
                        }
                    ]
                },
                "MachineDetail": {
                    "allOf": [
                        { "$ref": "#/components/schemas/MachineSummary" },
                        {
                            "type": "object",
                            "properties": {
                                "latest": { "oneOf": [{ "$ref": "#/components/schemas/MachineSnapshot" }, { "type": "null" }] },
                                "history": { "type": "array", "items": { "$ref": "#/components/schemas/MachineSnapshotPoint" } }
                            }
                        }
                    ]
                },
                "TicketStatus": {
                    "type": "string",
                    "enum": ["open", "in_progress", "resolved", "closed", "cancelled"]
                },
                "TicketPriority": {
                    "type": "string",
                    "enum": ["urgent", "high", "normal", "low"]
                },
                "TicketCategory": {
                    "type": "string",
                    "enum": ["machine_health", "device_offline", "delivery_failed", "distribution_failed", "security", "other"]
                },
                "TicketSource": {
                    "type": "string",
                    "enum": ["auto", "manual"]
                },
                "TicketEventKind": {
                    "type": "string",
                    "enum": ["created", "claimed", "assigned", "status_changed", "priority_changed", "commented", "occurred", "updated", "reminded"]
                },
                "Ticket": {
                    "type": "object",
                    "required": ["id", "code", "title", "description", "status", "priority", "category", "source",
                                 "source_type", "source_id", "dedupe_key", "area_id", "room_id", "serial",
                                 "device_slot", "reporter_id", "reporter_name", "assignee_id", "assignee_name",
                                 "due_at", "acknowledged_at", "resolved_at", "closed_at", "resolution",
                                 "root_cause", "occurrence_count", "last_seen_at", "metadata",
                                 "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string", "description": "人类可读单号 `T-YYYYMMDD-####`（只用于展示/搜索，路由用 id）。" },
                        "title": { "type": "string" },
                        "description": { "type": "string" },
                        "status": { "$ref": "#/components/schemas/TicketStatus" },
                        "priority": { "$ref": "#/components/schemas/TicketPriority" },
                        "category": { "$ref": "#/components/schemas/TicketCategory" },
                        "source": { "$ref": "#/components/schemas/TicketSource" },
                        "source_type": { "type": ["string", "null"], "description": "精确信号来源类型，如 machine_snapshot / device_presence / schedule_delivery。" },
                        "source_id": { "type": ["string", "null"], "description": "源行 id（快照 id / 投递 id 等），用于详情回看原始证据。" },
                        "dedupe_key": { "type": ["string", "null"], "description": "自动单幂等键；手工单为 null。" },
                        "area_id": { "type": ["string", "null"], "format": "uuid" },
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "serial": { "type": ["string", "null"] },
                        "device_slot": { "type": ["string", "null"] },
                        "reporter_id": { "type": ["string", "null"], "format": "uuid" },
                        "reporter_name": { "type": ["string", "null"] },
                        "assignee_id": { "type": ["string", "null"], "format": "uuid" },
                        "assignee_name": { "type": ["string", "null"] },
                        "due_at": { "type": ["string", "null"], "format": "date-time", "description": "按优先级计算的时效截止（urgent 1h / high 4h / normal 24h / low 72h）。" },
                        "acknowledged_at": { "type": ["string", "null"], "format": "date-time" },
                        "resolved_at": { "type": ["string", "null"], "format": "date-time" },
                        "closed_at": { "type": ["string", "null"], "format": "date-time" },
                        "resolution": { "type": ["string", "null"] },
                        "root_cause": { "oneOf": [{ "type": "string", "enum": ["hardware", "network", "software", "config", "usage", "other"] }, { "type": "null" }], "description": "根因分类（resolve/close 时写入；NULL = 未填写）。" },
                        "occurrence_count": { "type": "integer", "format": "int64" },
                        "last_seen_at": { "type": "string", "format": "date-time" },
                        "metadata": { "type": "object", "description": "建单时的信号快照（健康问题列表、投递错误等），源表被裁剪后仍可回看。" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "TicketPage": {
                    "type": "object",
                    "required": ["items", "total", "page", "page_size"],
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/Ticket" } },
                        "total": { "type": "integer", "format": "int64" },
                        "page": { "type": "integer", "format": "int64" },
                        "page_size": { "type": "integer", "format": "int64" }
                    }
                },
                "TicketEvent": {
                    "type": "object",
                    "required": ["id", "ticket_id", "kind", "actor_id", "actor_name", "from_status", "to_status",
                                 "note", "payload", "source_type", "source_id", "created_at"],
                    "properties": {
                        "id": { "type": "integer", "format": "int64" },
                        "ticket_id": { "type": "string", "format": "uuid" },
                        "kind": { "$ref": "#/components/schemas/TicketEventKind" },
                        "actor_id": { "type": ["string", "null"], "format": "uuid" },
                        "actor_name": { "type": "string", "description": "操作人显示名；系统事件固定为「系统」。" },
                        "from_status": { "oneOf": [{ "$ref": "#/components/schemas/TicketStatus" }, { "type": "null" }] },
                        "to_status": { "oneOf": [{ "$ref": "#/components/schemas/TicketStatus" }, { "type": "null" }] },
                        "note": { "type": "string" },
                        "payload": { "type": "object" },
                        "source_type": { "type": ["string", "null"] },
                        "source_id": { "type": ["string", "null"] },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "TicketContext": {
                    "type": "object",
                    "description": "详情页关联上下文（全部可空）。health_status/health_score/collected_at 取该 serial 最新 machine_snapshots；online = device-pki 会话在线或最近 5 分钟内有快照，无法判断为 null。",
                    "required": ["room_id", "room_name", "room_code", "area_id", "area_name", "serial",
                                 "device_slot", "health_status", "health_score", "collected_at", "online"],
                    "properties": {
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "room_name": { "type": ["string", "null"] },
                        "room_code": { "type": ["string", "null"] },
                        "area_id": { "type": ["string", "null"], "format": "uuid" },
                        "area_name": { "type": ["string", "null"] },
                        "serial": { "type": ["string", "null"] },
                        "device_slot": { "type": ["string", "null"] },
                        "health_status": { "type": ["string", "null"], "enum": ["ok", "warning", "critical", null] },
                        "health_score": { "type": ["integer", "null"] },
                        "collected_at": { "type": ["string", "null"], "format": "date-time" },
                        "online": { "type": ["boolean", "null"] }
                    }
                },
                "TicketDetail": {
                    "type": "object",
                    "required": ["ticket", "events", "context"],
                    "properties": {
                        "ticket": { "$ref": "#/components/schemas/Ticket" },
                        "events": { "type": "array", "items": { "$ref": "#/components/schemas/TicketEvent" } },
                        "context": { "$ref": "#/components/schemas/TicketContext" }
                    }
                },
                "TicketSummary": {
                    "type": "object",
                    "required": ["by_status", "unassigned", "overdue", "mine_active"],
                    "properties": {
                        "by_status": {
                            "type": "object",
                            "description": "五种状态的全量计数（缺失状态补 0）。",
                            "additionalProperties": { "type": "integer", "format": "int64" }
                        },
                        "unassigned": { "type": "integer", "format": "int64" },
                        "overdue": { "type": "integer", "format": "int64" },
                        "mine_active": { "type": "integer", "format": "int64" }
                    }
                },
                "TicketAssignee": {
                    "type": "object",
                    "required": ["id", "display_name", "roles"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "display_name": { "type": "string" },
                        "roles": { "type": "array", "items": { "type": "string" } }
                    }
                },
                "TicketRule": {
                    "type": "object",
                    "required": ["id", "name", "signal_type", "enabled", "priority", "scope_type",
                                 "scope_id", "thresholds", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "name": { "type": "string" },
                        "signal_type": {
                            "type": "string",
                            "enum": ["machine_health", "device_offline", "schedule_delivery",
                                     "presence_delivery", "transfer", "image_verify"]
                        },
                        "enabled": { "type": "boolean" },
                        "priority": { "$ref": "#/components/schemas/TicketPriority" },
                        "scope_type": { "type": "string", "enum": ["global", "area", "room"] },
                        "scope_id": { "type": ["string", "null"], "format": "uuid" },
                        "thresholds": { "type": "object", "additionalProperties": { "type": "integer", "format": "int64" } },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "TicketRuleList": {
                    "type": "object",
                    "required": ["items", "total"],
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/TicketRule" } },
                        "total": { "type": "integer", "format": "int64" }
                    }
                },
                "CreateTicketRuleRequest": {
                    "type": "object",
                    "required": ["name", "signal_type"],
                    "properties": {
                        "name": { "type": "string" },
                        "signal_type": {
                            "type": "string",
                            "enum": ["machine_health", "device_offline", "schedule_delivery",
                                     "presence_delivery", "transfer", "image_verify"]
                        },
                        "enabled": { "type": "boolean", "default": true },
                        "priority": { "$ref": "#/components/schemas/TicketPriority" },
                        "scope_type": { "type": "string", "enum": ["global", "area", "room"], "default": "global" },
                        "scope_id": { "type": ["string", "null"], "format": "uuid" },
                        "thresholds": { "type": "object", "additionalProperties": { "type": "integer", "format": "int64" } }
                    }
                },
                "UpdateTicketRuleRequest": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "enabled": { "type": "boolean" },
                        "priority": { "$ref": "#/components/schemas/TicketPriority" },
                        "scope_type": { "type": "string", "enum": ["global", "area", "room"] },
                        "scope_id": { "type": ["string", "null"], "format": "uuid", "description": "显式 null 仅在 scope_type=global 时合法。" },
                        "thresholds": { "type": "object", "additionalProperties": { "type": "integer", "format": "int64" } }
                    }
                },
                "TicketStreamEvent": {
                    "type": "object",
                    "required": ["id", "ticket_id", "ticket_code", "kind", "status",
                                 "actor_id", "actor_name", "note", "payload", "created_at"],
                    "properties": {
                        "id": { "type": "integer", "format": "int64" },
                        "ticket_id": { "type": "string", "format": "uuid" },
                        "ticket_code": { "type": "string" },
                        "kind": { "$ref": "#/components/schemas/TicketEventKind" },
                        "status": { "$ref": "#/components/schemas/TicketStatus" },
                        "actor_id": { "type": ["string", "null"], "format": "uuid" },
                        "actor_name": { "type": "string" },
                        "note": { "type": "string" },
                        "payload": { "type": "object" },
                        "created_at": { "type": "string", "format": "date-time" }
                    }
                },
                "TicketStatsTotals": {
                    "type": "object",
                    "required": ["created", "resolved", "closed", "open_now", "overdue", "avg_resolve_secs", "median_resolve_secs"],
                    "properties": {
                        "created": { "type": "integer", "format": "int64" },
                        "resolved": { "type": "integer", "format": "int64" },
                        "closed": { "type": "integer", "format": "int64" },
                        "open_now": { "type": "integer", "format": "int64" },
                        "overdue": { "type": "integer", "format": "int64" },
                        "avg_resolve_secs": { "type": ["integer", "null"], "format": "int64" },
                        "median_resolve_secs": { "type": ["integer", "null"], "format": "int64" }
                    }
                },
                "TicketDayStat": {
                    "type": "object",
                    "required": ["date", "created", "resolved", "closed"],
                    "properties": {
                        "date": { "type": "string", "description": "按关联区域时区聚合的本地日期 YYYY-MM-DD。" },
                        "created": { "type": "integer", "format": "int64" },
                        "resolved": { "type": "integer", "format": "int64" },
                        "closed": { "type": "integer", "format": "int64" }
                    }
                },
                "TicketCategoryStat": {
                    "type": "object",
                    "required": ["category", "count"],
                    "properties": {
                        "category": { "$ref": "#/components/schemas/TicketCategory" },
                        "count": { "type": "integer", "format": "int64" }
                    }
                },
                "TicketPriorityStat": {
                    "type": "object",
                    "required": ["priority", "count"],
                    "properties": {
                        "priority": { "$ref": "#/components/schemas/TicketPriority" },
                        "count": { "type": "integer", "format": "int64" }
                    }
                },
                "TicketTopRoom": {
                    "type": "object",
                    "required": ["room_id", "room_name", "count"],
                    "properties": {
                        "room_id": { "type": "string", "format": "uuid" },
                        "room_name": { "type": "string" },
                        "count": { "type": "integer", "format": "int64" }
                    }
                },
                "TicketTopDevice": {
                    "type": "object",
                    "required": ["serial", "device_slot", "count"],
                    "properties": {
                        "serial": { "type": "string" },
                        "device_slot": { "type": ["string", "null"] },
                        "count": { "type": "integer", "format": "int64" }
                    }
                },
                "TicketAssigneeStat": {
                    "type": "object",
                    "required": ["assignee_id", "display_name", "active", "resolved", "avg_resolve_secs"],
                    "properties": {
                        "assignee_id": { "type": "string", "format": "uuid" },
                        "display_name": { "type": "string" },
                        "active": { "type": "integer", "format": "int64" },
                        "resolved": { "type": "integer", "format": "int64" },
                        "avg_resolve_secs": { "type": ["integer", "null"], "format": "int64" }
                    }
                },
                "TicketStats": {
                    "type": "object",
                    "required": ["from", "to", "totals", "by_day", "by_category", "by_priority",
                                 "top_rooms", "top_devices", "by_assignee"],
                    "properties": {
                        "from": { "type": "string", "format": "date-time" },
                        "to": { "type": "string", "format": "date-time" },
                        "totals": { "$ref": "#/components/schemas/TicketStatsTotals" },
                        "by_day": { "type": "array", "items": { "$ref": "#/components/schemas/TicketDayStat" } },
                        "by_category": { "type": "array", "items": { "$ref": "#/components/schemas/TicketCategoryStat" } },
                        "by_priority": { "type": "array", "items": { "$ref": "#/components/schemas/TicketPriorityStat" } },
                        "top_rooms": { "type": "array", "items": { "$ref": "#/components/schemas/TicketTopRoom" } },
                        "top_devices": { "type": "array", "items": { "$ref": "#/components/schemas/TicketTopDevice" } },
                        "by_assignee": { "type": "array", "items": { "$ref": "#/components/schemas/TicketAssigneeStat" } }
                    }
                },
                "CreateTicketRequest": {
                    "type": "object",
                    "required": ["title"],
                    "properties": {
                        "title": { "type": "string", "description": "1-200 字符。" },
                        "description": { "type": "string", "description": "纯文本，最长 10000 字符。" },
                        "category": { "$ref": "#/components/schemas/TicketCategory" },
                        "priority": { "$ref": "#/components/schemas/TicketPriority" },
                        "room_id": { "type": ["string", "null"], "format": "uuid" },
                        "area_id": { "type": ["string", "null"], "format": "uuid" },
                        "serial": { "type": ["string", "null"] },
                        "device_slot": { "type": ["string", "null"] }
                    }
                },
                "UpdateTicketRequest": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "description": { "type": ["string", "null"], "description": "缺失 = 不修改；显式 null = 清空。" },
                        "priority": { "$ref": "#/components/schemas/TicketPriority" },
                        "category": { "$ref": "#/components/schemas/TicketCategory" }
                    }
                },
                "AssignTicketRequest": {
                    "type": "object",
                    "required": ["assignee_id"],
                    "properties": {
                        "assignee_id": { "type": ["string", "null"], "format": "uuid", "description": "null = 释放认领；目标必须拥有 ticket:handle。" }
                    }
                },
                "TransitionTicketRequest": {
                    "type": "object",
                    "required": ["to_status"],
                    "properties": {
                        "to_status": { "$ref": "#/components/schemas/TicketStatus" },
                        "resolution": { "type": ["string", "null"], "description": "标记解决（to_status=resolved）时必填。" },
                        "note": { "type": ["string", "null"] },
                        "root_cause": { "oneOf": [{ "type": "string", "enum": ["hardware", "network", "software", "config", "usage", "other"] }, { "type": "null" }], "description": "根因分类（resolve/close 时写入；resolved→closed 可补充）。" }
                    }
                },
                "TicketScopeUser": {
                    "type": "object",
                    "required": ["user_id", "display_name", "roles", "area_ids"],
                    "properties": {
                        "user_id": { "type": "string", "format": "uuid" },
                        "display_name": { "type": "string" },
                        "roles": { "type": "array", "items": { "type": "string" } },
                        "area_ids": { "type": "array", "items": { "type": "string", "format": "uuid" } }
                    }
                },
                "ScopeAreaOption": {
                "type": "object",
                "required": ["id", "name", "code"],
                "properties": {
                    "id": { "type": "string", "format": "uuid" },
                    "name": { "type": "string" },
                    "code": { "type": "string" }
                }
            },
            "UpdateTicketScopesRequest": {
                    "type": "object",
                    "required": ["area_ids"],
                    "properties": {
                        "area_ids": { "type": "array", "items": { "type": "string", "format": "uuid" }, "description": "整体替换该用户的区域范围；空数组 = 清除（恢复不限制）。" }
                    }
                },
                "TicketAttachment": {
                    "type": "object",
                    "required": ["id", "ticket_id", "object_key", "filename", "content_type", "size",
                                 "sha256", "uploaded_by", "uploaded_by_name", "created_at", "download_url"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "ticket_id": { "type": "string", "format": "uuid" },
                        "object_key": { "type": "string" },
                        "filename": { "type": "string" },
                        "content_type": { "type": "string" },
                        "size": { "type": "integer", "format": "int64" },
                        "sha256": { "type": "string" },
                        "uploaded_by": { "type": ["string", "null"], "format": "uuid" },
                        "uploaded_by_name": { "type": ["string", "null"] },
                        "created_at": { "type": "string", "format": "date-time" },
                        "download_url": { "type": ["string", "null"], "description": "下载预签名地址；未配置对象存储时为 null。" }
                    }
                },
                "AttachmentPresignRequest": {
                    "type": "object",
                    "required": ["filename", "content_type", "size", "sha256"],
                    "properties": {
                        "filename": { "type": "string", "description": "≤255 字符；对象 key 中会做安全字符替换。" },
                        "content_type": { "type": "string", "description": "image/*, application/pdf, text/plain, application/zip, application/gzip, application/octet-stream。" },
                        "size": { "type": "integer", "format": "int64", "description": "1..=20971520 字节。" },
                        "sha256": { "type": "string", "description": "64 位十六进制。" }
                    }
                },
                "AttachmentPresignReply": {
                    "type": "object",
                    "required": ["object_key", "upload_url", "expires_at"],
                    "properties": {
                        "object_key": { "type": "string" },
                        "upload_url": { "type": "string" },
                        "expires_at": { "type": "string", "format": "date-time" }
                    }
                },
                "AttachmentRegisterRequest": {
                    "type": "object",
                    "required": ["object_key", "filename", "content_type", "size", "sha256"],
                    "properties": {
                        "object_key": { "type": "string", "description": "必须是 presign 返回的 key（tickets/{ticket_id}/…）。" },
                        "filename": { "type": "string" },
                        "content_type": { "type": "string" },
                        "size": { "type": "integer", "format": "int64" },
                        "sha256": { "type": "string" }
                    }
                },
                "InspectionScopeType": {
                    "type": "string",
                    "enum": ["global", "area", "room"]
                },
                "InspectionRunStatus": {
                    "type": "string",
                    "enum": ["running", "ok", "issues", "failed"]
                },
                "InspectionLastRun": {
                    "type": "object",
                    "required": ["status", "started_at", "issue_machines", "ticket_id"],
                    "properties": {
                        "status": { "$ref": "#/components/schemas/InspectionRunStatus" },
                        "started_at": { "type": "string", "format": "date-time" },
                        "issue_machines": { "type": "integer", "format": "int64" },
                        "ticket_id": { "type": ["string", "null"], "format": "uuid" }
                    }
                },
                "InspectionPlan": {
                    "type": "object",
                    "required": ["id", "name", "scope_type", "scope_id", "interval_hours", "enabled",
                                 "created_by", "created_by_name", "last_run_at", "next_run_at",
                                 "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "name": { "type": "string" },
                        "scope_type": { "$ref": "#/components/schemas/InspectionScopeType" },
                        "scope_id": { "type": ["string", "null"], "format": "uuid" },
                        "interval_hours": { "type": "integer", "format": "int64", "description": "1..=168 小时。" },
                        "enabled": { "type": "boolean" },
                        "created_by": { "type": ["string", "null"], "format": "uuid" },
                        "created_by_name": { "type": ["string", "null"] },
                        "last_run_at": { "type": ["string", "null"], "format": "date-time" },
                        "next_run_at": { "type": ["string", "null"], "format": "date-time", "description": "null = 立即到期。" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "InspectionPlanView": {
                    "allOf": [
                        { "$ref": "#/components/schemas/InspectionPlan" },
                        {
                            "type": "object",
                            "required": ["last_run"],
                            "properties": {
                                "last_run": { "oneOf": [{ "$ref": "#/components/schemas/InspectionLastRun" }, { "type": "null" }] }
                            }
                        }
                    ]
                },
                "CreateInspectionPlanRequest": {
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": { "type": "string", "description": "1-100 字符。" },
                        "scope_type": { "$ref": "#/components/schemas/InspectionScopeType" },
                        "scope_id": { "type": ["string", "null"], "format": "uuid", "description": "scope_type=area/room 时必填且必须存在。" },
                        "interval_hours": { "type": "integer", "format": "int64", "minimum": 1, "maximum": 168 },
                        "enabled": { "type": "boolean" }
                    }
                },
                "UpdateInspectionPlanRequest": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "scope_type": { "$ref": "#/components/schemas/InspectionScopeType" },
                        "scope_id": { "type": ["string", "null"], "format": "uuid", "description": "缺失 = 不修改；显式 null 清空。" },
                        "interval_hours": { "type": "integer", "format": "int64", "minimum": 1, "maximum": 168 },
                        "enabled": { "type": "boolean" }
                    }
                },
                "InspectionRun": {
                    "type": "object",
                    "required": ["id", "plan_id", "started_at", "finished_at", "status",
                                 "checked_machines", "issue_machines", "ticket_id", "summary", "error"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "plan_id": { "type": "string", "format": "uuid" },
                        "started_at": { "type": "string", "format": "date-time" },
                        "finished_at": { "type": ["string", "null"], "format": "date-time" },
                        "status": { "$ref": "#/components/schemas/InspectionRunStatus" },
                        "checked_machines": { "type": "integer", "format": "int64" },
                        "issue_machines": { "type": "integer", "format": "int64" },
                        "ticket_id": { "type": ["string", "null"], "format": "uuid" },
                        "summary": { "type": "object", "description": "检查机器清单与异常明细快照。" },
                        "error": { "type": ["string", "null"] }
                    }
                },
                "CommentTicketRequest": {
                    "type": "object",
                    "required": ["body"],
                    "properties": {
                        "body": { "type": "string", "description": "评论内容（1-2000 字符），兼容别名 note。" }
                    }
                },
                "TermView": {
                    "type": "object",
                    "required": ["id", "code", "name", "start_date", "end_date", "is_active", "usage", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "start_date": { "type": "string", "format": "date" },
                        "end_date": { "type": "string", "format": "date" },
                        "is_active": { "type": "boolean" },
                        "usage": { "type": "integer", "format": "int64", "description": "被排课计划引用数（列表展示用）。" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "TeacherView": {
                    "type": "object",
                    "required": ["id", "code", "name", "phone", "email", "is_active", "metadata", "usage", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "phone": { "type": "string" },
                        "email": { "type": "string" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": "object" },
                        "usage": { "type": "integer", "format": "int64", "description": "被计划条目引用数（列表展示用）。" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "ClassView": {
                    "type": "object",
                    "required": ["id", "code", "name", "grade", "is_active", "metadata", "usage", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "grade": { "type": "string" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": "object" },
                        "usage": { "type": "integer", "format": "int64", "description": "被计划条目引用数（列表展示用）。" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "CourseView": {
                    "type": "object",
                    "required": ["id", "code", "name", "color", "is_active", "metadata", "usage", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "color": { "type": "string", "description": "前端课程色值（如 #3a5bd9）；空 = 按课程名取色。" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": "object" },
                        "usage": { "type": "integer", "format": "int64", "description": "被计划条目引用数（列表展示用）。" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "PlanStatus": {
                    "type": "string",
                    "enum": ["draft", "published"],
                    "description": "排课计划状态：draft 草稿可编辑 / published 已发布只读（同一学期只允许一个 published）。"
                },
                "SchedulePlan": {
                    "type": "object",
                    "required": ["id", "term_id", "term_name", "name", "status", "version", "published_at", "published_by", "created_by", "entry_count", "course_count", "room_count", "created_at", "updated_at"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "term_id": { "type": "string", "format": "uuid" },
                        "term_name": { "type": "string" },
                        "name": { "type": "string" },
                        "status": { "$ref": "#/components/schemas/PlanStatus" },
                        "version": { "type": "integer", "minimum": 1 },
                        "published_at": { "type": ["string", "null"], "format": "date-time" },
                        "published_by": { "type": ["string", "null"], "format": "uuid" },
                        "created_by": { "type": ["string", "null"], "format": "uuid" },
                        "entry_count": { "type": "integer", "format": "int64" },
                        "course_count": { "type": "integer", "format": "int64" },
                        "room_count": { "type": "integer", "format": "int64" },
                        "created_at": { "type": "string", "format": "date-time" },
                        "updated_at": { "type": "string", "format": "date-time" }
                    }
                },
                "PlanEntry": {
                    "type": "object",
                    "required": ["id", "plan_id", "room_id", "room_code", "room_name", "area_id", "area_name", "day_of_week", "start_time", "end_time", "course_id", "course_code", "course_name", "teacher_id", "teacher_name", "class_id", "class_name", "note"],
                    "properties": {
                        "id": { "type": "string", "format": "uuid" },
                        "plan_id": { "type": "string", "format": "uuid" },
                        "room_id": { "type": "string", "format": "uuid" },
                        "room_code": { "type": "string" },
                        "room_name": { "type": "string" },
                        "area_id": { "type": "string", "format": "uuid" },
                        "area_name": { "type": "string" },
                        "day_of_week": { "type": "integer", "minimum": 1, "maximum": 7, "description": "ISO 星期：周一=1 .. 周日=7。" },
                        "start_time": { "type": "string", "example": "08:00" },
                        "end_time": { "type": "string", "example": "09:40" },
                        "course_id": { "type": "string", "format": "uuid" },
                        "course_code": { "type": "string" },
                        "course_name": { "type": "string" },
                        "teacher_id": { "type": ["string", "null"], "format": "uuid" },
                        "teacher_name": { "type": ["string", "null"] },
                        "class_id": { "type": ["string", "null"], "format": "uuid" },
                        "class_name": { "type": ["string", "null"] },
                        "note": { "type": "string" }
                    }
                },
                "PlanEntryInput": {
                    "type": "object",
                    "required": ["room_id", "day_of_week", "start_time", "end_time", "course_id"],
                    "properties": {
                        "room_id": { "type": "string", "format": "uuid" },
                        "day_of_week": { "type": "integer", "minimum": 1, "maximum": 7, "description": "ISO 星期：周一=1 .. 周日=7。" },
                        "start_time": { "type": "string", "example": "08:00" },
                        "end_time": { "type": "string", "example": "09:40" },
                        "course_id": { "type": "string", "format": "uuid" },
                        "teacher_id": { "type": ["string", "null"], "format": "uuid", "description": "可选；给出的教师必须存在且启用。" },
                        "class_id": { "type": ["string", "null"], "format": "uuid", "description": "可选；给出的班级必须存在且启用。" },
                        "note": { "type": "string", "description": "备注，最长 1000 字符；缺省空。" }
                    }
                },
                "TermPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/TermView" } } ,
                        "total": { "type": "integer" } ,
                        "page": { "type": "integer" } ,
                        "page_size": { "type": "integer" }
                    }
                },
                "TeacherPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/TeacherView" } } ,
                        "total": { "type": "integer" } ,
                        "page": { "type": "integer" } ,
                        "page_size": { "type": "integer" }
                    }
                },
                "ClassPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/ClassView" } } ,
                        "total": { "type": "integer" } ,
                        "page": { "type": "integer" } ,
                        "page_size": { "type": "integer" }
                    }
                },
                "CoursePage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/CourseView" } } ,
                        "total": { "type": "integer" } ,
                        "page": { "type": "integer" } ,
                        "page_size": { "type": "integer" }
                    }
                },
                "SchedulePlanPage": {
                    "type": "object",
                    "properties": {
                        "items": { "type": "array", "items": { "$ref": "#/components/schemas/SchedulePlan" } } ,
                        "total": { "type": "integer" } ,
                        "page": { "type": "integer" } ,
                        "page_size": { "type": "integer" }
                    }
                },
                "PlanDetailResponse": {
                    "type": "object",
                    "required": ["plan", "entries"],
                    "properties": {
                        "plan": { "$ref": "#/components/schemas/SchedulePlan" } ,
                        "entries": { "type": "array", "items": { "$ref": "#/components/schemas/PlanEntry" }, "description": "按房间 → 星期 → 开始时间排序。" }
                    }
                },
                "PublishResultResponse": {
                    "type": "object",
                    "required": ["plan", "schedules_rebuilt", "slots_written", "occurrences"],
                    "properties": {
                        "plan": { "$ref": "#/components/schemas/SchedulePlan" } ,
                        "schedules_rebuilt": { "type": "integer", "description": "成功重建实例的 legacy 课表数（失败只记日志，不计入）。" } ,
                        "slots_written": { "type": "integer", "description": "写入 legacy 周模板的槽位数。" } ,
                        "occurrences": { "type": "integer", "description": "重建出的排课实例数。" }
                    }
                },
                "CreateTermRequest": {
                    "type": "object",
                    "required": ["code", "name", "start_date", "end_date"],
                    "properties": {
                        "code": { "type": "string", "description": "2-64 位，首字符字母，可含字母数字 - _ ." },
                        "name": { "type": "string", "maxLength": 200 },
                        "start_date": { "type": "string", "format": "date" },
                        "end_date": { "type": "string", "format": "date", "description": "必须 >= start_date。" },
                        "is_active": { "type": "boolean", "default": true }
                    }
                },
                "UpdateTermRequest": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "start_date": { "type": "string", "format": "date" },
                        "end_date": { "type": "string", "format": "date" },
                        "is_active": { "type": "boolean" }
                    }
                },
                "CreateTeacherRequest": {
                    "type": "object",
                    "required": ["code", "name"],
                    "properties": {
                        "code": { "type": "string", "description": "2-64 位，首字符字母，可含字母数字 - _ ." },
                        "name": { "type": "string", "maxLength": 200 },
                        "phone": { "type": "string", "maxLength": 32 },
                        "email": { "type": "string" },
                        "is_active": { "type": "boolean", "default": true },
                        "metadata": { "type": "object" }
                    }
                },
                "UpdateTeacherRequest": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "phone": { "type": "string" },
                        "email": { "type": "string" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": ["object", "null"], "description": "缺失 = 不修改；显式 null = 重置为 {}。" }
                    }
                },
                "CreateClassRequest": {
                    "type": "object",
                    "required": ["code", "name"],
                    "properties": {
                        "code": { "type": "string", "description": "2-64 位，首字符字母，可含字母数字 - _ ." },
                        "name": { "type": "string", "maxLength": 200 },
                        "grade": { "type": "string", "maxLength": 64 },
                        "is_active": { "type": "boolean", "default": true },
                        "metadata": { "type": "object" }
                    }
                },
                "UpdateClassRequest": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "grade": { "type": "string" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": ["object", "null"], "description": "缺失 = 不修改；显式 null = 重置为 {}。" }
                    }
                },
                "CreateCourseRequest": {
                    "type": "object",
                    "required": ["code", "name"],
                    "properties": {
                        "code": { "type": "string", "description": "2-64 位，首字符字母，可含字母数字 - _ ." },
                        "name": { "type": "string", "maxLength": 200 },
                        "color": { "type": "string", "maxLength": 32, "description": "前端课程色值；空 = 按课程名取色。" },
                        "is_active": { "type": "boolean", "default": true },
                        "metadata": { "type": "object" }
                    }
                },
                "UpdateCourseRequest": {
                    "type": "object",
                    "properties": {
                        "code": { "type": "string" },
                        "name": { "type": "string" },
                        "color": { "type": "string" },
                        "is_active": { "type": "boolean" },
                        "metadata": { "type": ["object", "null"], "description": "缺失 = 不修改；显式 null = 重置为 {}。" }
                    }
                },
                "CreateSchedulePlanRequest": {
                    "type": "object",
                    "required": ["term_id", "name"],
                    "properties": {
                        "term_id": { "type": "string", "format": "uuid", "description": "计划归属的学期；必须存在且启用。" },
                        "name": { "type": "string", "maxLength": 200 }
                    }
                },
                "UpdateSchedulePlanRequest": {
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": { "type": "string", "maxLength": 200 }
                    }
                },
                "ReplacePlanEntriesRequest": {
                    "type": "object",
                    "required": ["entries"],
                    "properties": {
                        "entries": { "type": "array", "items": { "$ref": "#/components/schemas/PlanEntryInput" }, "maxItems": 2000, "description": "全量替换；同一房间同一天的时段不得重叠。" }
                    }
                }

            }
        },
        "paths": {
            "/auth/login": {
                "post": {
                    "tags": ["Auth"],
                    "summary": "登录",
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/LoginRequest" } } } },
                    "responses": {
                        "200": { "description": "登录成功", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TokenResponse" } } } },
                        "401": error_response("用户名或密码错误"),
                        "429": error_response("登录失败次数过多，账号临时锁定")
                    }
                }
            },
            "/auth/refresh": {
                "post": {
                    "tags": ["Auth"],
                    "summary": "轮换 refresh token",
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RefreshRequest" } } } },
                    "responses": { "200": { "description": "新令牌", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TokenResponse" } } } }, "401": error_response("refresh token 无效") }
                }
            },
            "/auth/logout": {
                "post": {
                    "tags": ["Auth"],
                    "summary": "注销 refresh token",
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RefreshRequest" } } } },
                    "responses": { "204": { "description": "已注销" } }
                }
            },
            "/auth/me": {
                "get": {
                    "tags": ["Auth"],
                    "summary": "当前用户",
                    "security": secured(),
                    "responses": { "200": { "description": "当前用户", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UserProfile" } } } }, "401": error_response("未登录") }
                }
            },
            "/auth/password/forgot": {
                "post": {
                    "tags": ["Auth"],
                    "summary": "请求密码重置邮件",
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["email"], "properties": { "email": { "type": "string", "format": "email" } } } } } },
                    "responses": { "204": { "description": "已受理；无论邮箱是否存在都返回 204" } }
                }
            },
            "/auth/password/reset": {
                "post": {
                    "tags": ["Auth"],
                    "summary": "使用一次性令牌重置密码",
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["token", "new_password"], "properties": { "token": { "type": "string" }, "new_password": { "type": "string", "format": "password" } } } } } },
                    "responses": { "204": { "description": "密码已重置，所有设备会话已撤销" }, "400": error_response("令牌无效或密码不合规") }
                }
            },
            "/auth/email/verification": {
                "post": {
                    "tags": ["Auth"],
                    "summary": "请求邮箱验证邮件",
                    "security": secured(),
                    "responses": { "204": { "description": "验证邮件已发送（开发环境写日志）" }, "400": error_response("账号未设置邮箱") }
                }
            },
            "/auth/email/verify": {
                "post": {
                    "tags": ["Auth"],
                    "summary": "验证邮箱",
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "required": ["token"], "properties": { "token": { "type": "string" } } } } } },
                    "responses": { "204": { "description": "邮箱已验证" }, "400": error_response("验证令牌无效或已过期") }
                }
            },
            "/auth/sessions": {
                "get": {
                    "tags": ["Auth"],
                    "summary": "列出当前用户的设备会话",
                    "security": secured(),
                    "responses": { "200": { "description": "会话列表", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/UserSession" } } } } } }
                },
                "delete": {
                    "tags": ["Auth"],
                    "summary": "撤销当前用户全部设备会话",
                    "security": secured(),
                    "responses": { "204": { "description": "已撤销" } }
                }
            },
            "/auth/sessions/{id}": {
                "delete": {
                    "tags": ["Auth"],
                    "summary": "撤销指定设备会话",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "204": { "description": "已撤销" }, "404": error_response("会话不存在") }
                }
            },
            "/announcements": {
                "get": {
                    "tags": ["Announcements"],
                    "summary": "公开公告列表",
                    "parameters": [
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "已发布公告", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AnnouncementPage" } } } } }
                }
            },
            "/announcements/{id}": {
                "get": {
                    "tags": ["Announcements"],
                    "summary": "公开公告详情",
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "200": { "description": "公告", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Announcement" } } } }, "404": error_response("公告不存在") }
                }
            },
            "/admin/announcements": {
                "get": {
                    "tags": ["Admin"],
                    "summary": "后台公告列表",
                    "security": secured(),
                    "parameters": [
                        { "name": "page", "in": "query", "schema": { "type": "integer" } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer" } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string" } },
                        { "name": "status", "in": "query", "schema": { "type": "string", "enum": ["draft", "published", "archived"] } }
                    ],
                    "responses": { "200": { "description": "公告分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AnnouncementPage" } } } }, "403": error_response("缺少 announcement:read") }
                },
                "post": {
                    "tags": ["Admin"],
                    "summary": "创建公告",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AnnouncementInput" } } } },
                    "responses": { "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Announcement" } } } }, "403": error_response("缺少公告写权限") }
                }
            },
            "/admin/announcements/{id}": {
                "patch": {
                    "tags": ["Admin"],
                    "summary": "更新公告",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AnnouncementInput" } } } },
                    "responses": { "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Announcement" } } } } }
                },
                "delete": {
                    "tags": ["Admin"],
                    "summary": "删除公告",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "204": { "description": "已删除" } }
                }
            },
            "/admin/users": {
                "get": {
                    "tags": ["Admin"],
                    "summary": "用户列表",
                    "security": secured(),
                    "parameters": [
                        { "name": "page", "in": "query", "schema": { "type": "integer" } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer" } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string" } }
                    ],
                    "responses": { "200": { "description": "用户分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UserPage" } } } } }
                },
                "post": {
                    "tags": ["Admin"],
                    "summary": "创建用户",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateUserRequest" } } } },
                    "responses": { "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UserSummary" } } } } }
                }
            },
            "/admin/users/{id}": {
                "patch": {
                    "tags": ["Admin"],
                    "summary": "更新用户",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateUserRequest" } } } },
                    "responses": { "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UserSummary" } } } } }
                }
            },
            "/admin/users/{id}/roles": {
                "put": {
                    "tags": ["Admin"],
                    "summary": "设置用户角色",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SetRolesRequest" } } } },
                    "responses": { "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UserSummary" } } } } }
                }
            },
            "/admin/roles": {
                "get": {
                    "tags": ["Admin"],
                    "summary": "角色列表",
                    "security": secured(),
                    "responses": { "200": { "description": "角色列表", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/RoleSummary" } } } } } }
                },
                "post": {
                    "tags": ["Admin"],
                    "summary": "创建角色",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateRoleRequest" } } } },
                    "responses": { "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RoleSummary" } } } } }
                }
            },
            "/admin/roles/{id}": {
                "delete": {
                    "tags": ["Admin"],
                    "summary": "删除角色",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": { "204": { "description": "已删除" } }
                }
            },
            "/admin/roles/{id}/permissions": {
                "put": {
                    "tags": ["Admin"],
                    "summary": "设置角色权限",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SetPermissionsRequest" } } } },
                    "responses": { "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RoleSummary" } } } } }
                }
            },
            "/admin/permissions": {
                "get": {
                    "tags": ["Admin"],
                    "summary": "权限列表",
                    "security": secured(),
                    "responses": { "200": { "description": "权限列表", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/Permission" } } } } } }
                }
            },
            "/admin/audit-logs": {
                "get": {
                    "tags": ["Admin"],
                    "summary": "审计日志",
                    "security": secured(),
                    "parameters": [
                        { "name": "action", "in": "query", "schema": { "type": "string" } },
                        { "name": "resource_type", "in": "query", "schema": { "type": "string" } },
                        { "name": "page", "in": "query", "schema": { "type": "integer" } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer" } }
                    ],
                    "responses": { "200": { "description": "审计分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AuditLogPage" } } } }, "403": error_response("缺少 audit:read") }
                }
            },
            "/admin/areas": {
                "get": {
                    "tags": ["Areas"],
                    "summary": "区域分页",
                    "security": secured(),
                    "parameters": [
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string" } },
                        { "name": "include_inactive", "in": "query", "schema": { "type": "boolean", "default": false } }
                    ],
                    "responses": {
                        "200": { "description": "区域分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AreaPage" } } } },
                        "403": error_response("缺少 area:read")
                    }
                },
                "post": {
                    "tags": ["Areas"],
                    "summary": "创建区域（可选 IANA timezone，缺省 Asia/Shanghai）",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateAreaRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Area" } } } },
                        "400": error_response("参数校验失败"),
                        "403": error_response("缺少 area:manage"),
                        "409": error_response("区域编码已存在")
                    }
                }
            },
            "/admin/areas/{id}": {
                "get": {
                    "tags": ["Areas"],
                    "summary": "区域详情",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "区域", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Area" } } } },
                        "404": error_response("区域不存在")
                    }
                },
                "patch": {
                    "tags": ["Areas"],
                    "summary": "局部更新区域（parent_id / metadata 可传 null；timezone 变更后重建区域实例）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateAreaRequest" } } } },
                    "responses": {
                        "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Area" } } } },
                        "400": error_response("参数校验失败 / 没有需要更新的字段"),
                        "404": error_response("区域不存在")
                    }
                },
                "delete": {
                    "tags": ["Areas"],
                    "summary": "删除区域（仍有房间时 409）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "404": error_response("区域不存在"),
                        "409": error_response("区域下仍有房间")
                    }
                }
            },
            "/admin/rooms": {
                "get": {
                    "tags": ["Rooms"],
                    "summary": "房间分页",
                    "security": secured(),
                    "parameters": [
                        { "name": "area_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string" } },
                        { "name": "include_inactive", "in": "query", "schema": { "type": "boolean", "default": false } }
                    ],
                    "responses": {
                        "200": { "description": "房间分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RoomPage" } } } },
                        "403": error_response("缺少 room:read")
                    }
                },
                "post": {
                    "tags": ["Rooms"],
                    "summary": "创建房间",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateRoomRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Room" } } } },
                        "400": error_response("参数校验失败"),
                        "403": error_response("缺少 room:manage"),
                        "409": error_response("房间编码或设备槽位已存在")
                    }
                }
            },
            "/admin/rooms/{id}": {
                "get": {
                    "tags": ["Rooms"],
                    "summary": "房间详情",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "房间", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Room" } } } },
                        "404": error_response("房间不存在")
                    }
                },
                "patch": {
                    "tags": ["Rooms"],
                    "summary": "局部更新房间（device_slot / webhook_url 可传 null 清空）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateRoomRequest" } } } },
                    "responses": {
                        "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Room" } } } },
                        "400": error_response("参数校验失败 / 没有需要更新的字段"),
                        "404": error_response("房间不存在")
                    }
                },
                "delete": {
                    "tags": ["Rooms"],
                    "summary": "删除房间（级联删除课表与触发规则）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "404": error_response("房间不存在")
                    }
                }
            },
            "/admin/devices": {
                "get": {
                    "tags": ["Devices"],
                    "summary": "设备分页（含房间/区域、注册状态与在线状态）",
                    "security": secured(),
                    "parameters": [
                        { "name": "area_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "room_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "status", "in": "query", "schema": { "$ref": "#/components/schemas/DeviceStatus" } },
                        { "name": "assigned", "in": "query", "schema": { "type": "boolean" }, "description": "true = 只看已绑定房间；false = 只看未绑定。" },
                        { "name": "keyword", "in": "query", "schema": { "type": "string" }, "description": "模糊匹配 serial / device_slot / 房间编码 / 房间名称。" },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } }
                    ],
                    "responses": {
                        "200": { "description": "设备分页（按 issued_at 倒序）", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DevicePage" } } } },
                        "400": error_response("设备状态或分页参数不合法"),
                        "403": error_response("缺少 device:read")
                    }
                }
            },
            "/admin/devices/{serial}": {
                "get": {
                    "tags": ["Devices"],
                    "summary": "设备详情",
                    "security": secured(),
                    "parameters": [{ "name": "serial", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "设备详情", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DeviceInfo" } } } },
                        "403": error_response("缺少 device:read"),
                        "404": error_response("设备不存在")
                    }
                }
            },
            "/admin/rooms/{id}/device": {
                "get": {
                    "tags": ["Devices"],
                    "summary": "房间设备摘要（未绑定槽位返回 null）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "设备摘要或 null", "content": { "application/json": { "schema": { "oneOf": [{ "$ref": "#/components/schemas/RoomDevice" }, { "type": "null" }] } } } },
                        "403": error_response("缺少 device:read"),
                        "404": error_response("房间不存在")
                    }
                },
                "put": {
                    "tags": ["Devices"],
                    "summary": "绑定 / 迁移 / 解绑房间设备槽位（同一槽位自动从旧房间迁移）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": {
                            "type": "object",
                            "required": ["device_slot"],
                            "properties": { "device_slot": { "type": ["string", "null"], "description": "字符串 = 绑定/迁移；null = 解绑（缺失该字段为非法请求）。" } }
                        } } }
                    },
                    "responses": {
                        "200": { "description": "绑定后的房间（含 device 摘要）", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Room" } } } },
                        "400": error_response("设备槽位不合法 / 请求体缺少 device_slot"),
                        "403": error_response("缺少 device:manage"),
                        "404": error_response("房间不存在"),
                        "409": error_response("槽位并发绑定冲突")
                    }
                }
            },
            "/admin/rooms/{id}/device-token": {
                "post": {
                    "tags": ["Devices"],
                    "summary": "签发设备注册 token（房间无槽位时自动分配；明文 token 只返回一次）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DeviceTokenRequest" } } } },
                    "responses": {
                        "200": { "description": "注册 token 与注册链接", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DeviceTokenReply" } } } },
                        "400": error_response("token 类型或 ttl_secs 不合法"),
                        "403": error_response("缺少 device:manage"),
                        "404": error_response("房间不存在")
                    }
                }
            },
            "/admin/rooms/{id}/devices": {
                "get": {
                    "tags": ["Devices"],
                    "summary": "房间机器列表（主设备 + 子机）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "机器列表（主设备在前）", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/RoomMachine" } } } } },
                        "403": error_response("缺少 device:read"),
                        "404": error_response("房间不存在")
                    }
                }
            },
            "/admin/rooms/{id}/devices/token": {
                "post": {
                    "tags": ["Devices"],
                    "summary": "给房间新增一台子机并签发注册 token（槽位自动分配为 {room_code}-m{n}）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/MachineTokenRequest" } } } },
                    "responses": {
                        "200": { "description": "注册 token 与注册链接", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DeviceTokenReply" } } } },
                        "400": error_response("token 类型 / ttl_secs / label 不合法"),
                        "403": error_response("缺少 device:manage"),
                        "404": error_response("房间不存在")
                    }
                }
            },
            "/admin/rooms/{id}/devices/{slot}": {
                "delete": {
                    "tags": ["Devices"],
                    "summary": "解除房间与子机的绑定（不吊销证书）",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "slot", "in": "path", "required": true, "schema": { "type": "string" }, "description": "子机槽位（主设备请走 PUT /admin/rooms/{id}/device）。" }
                    ],
                    "responses": {
                        "204": { "description": "已解除绑定" },
                        "403": error_response("缺少 device:manage"),
                        "404": error_response("房间未绑定该子机")
                    }
                }
            },
            "/admin/rooms/{id}/occurrences": {
                "get": {
                    "tags": ["Schedules"],
                    "summary": "房间维度排课实例（UTC + 区域时区本地时间；默认按房间时区今天起 7 天）",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "from", "in": "query", "schema": { "type": "string", "format": "date" } },
                        { "name": "to", "in": "query", "schema": { "type": "string", "format": "date" } }
                    ],
                    "responses": {
                        "200": { "description": "排课实例", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/Occurrence" } } } } },
                        "400": error_response("日期区间不合法"),
                        "404": error_response("房间不存在")
                    }
                }
            },
            "/admin/rooms/{id}/event-rules": {
                "get": {
                    "tags": ["ScheduleEvents"],
                    "summary": "房间触发规则",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "触发规则", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/EventRule" } } } } },
                        "404": error_response("房间不存在")
                    }
                },
                "put": {
                    "tags": ["ScheduleEvents"],
                    "summary": "全量替换房间触发规则",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ReplaceEventRulesRequest" } } } },
                    "responses": {
                        "200": { "description": "已替换", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/EventRule" } } } } },
                        "400": error_response("规则校验失败"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("房间不存在")
                    }
                }
            },
            "/admin/schedules": {
                "get": {
                    "tags": ["Schedules"],
                    "summary": "课表分页",
                    "security": secured(),
                    "parameters": [
                        { "name": "room_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } }
                    ],
                    "responses": {
                        "200": { "description": "课表分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SchedulePage" } } } },
                        "403": error_response("缺少 schedule:read")
                    }
                },
                "post": {
                    "tags": ["Schedules"],
                    "summary": "创建课表",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateScheduleRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Schedule" } } } },
                        "400": error_response("参数校验失败"),
                        "403": error_response("缺少 schedule:manage")
                    }
                }
            },
            "/admin/schedule-board": {
                "get": {
                    "tags": ["ScheduleBoard"],
                    "summary": "排课工作台聚合数据（一次返回 areas / rooms / schedules / weekly_slots / exceptions）",
                    "security": secured(),
                    "parameters": [
                        { "name": "from", "in": "query", "required": true, "schema": { "type": "string", "format": "date" }, "description": "起始日期（YYYY-MM-DD，必填）。" },
                        { "name": "to", "in": "query", "required": true, "schema": { "type": "string", "format": "date" }, "description": "结束日期（YYYY-MM-DD，必填，to >= from，跨度 ≤ 62 天）。" },
                        { "name": "area_id", "in": "query", "schema": { "type": "string", "format": "uuid" }, "description": "只过滤 rooms，不裁剪 areas。" }
                    ],
                    "responses": {
                        "200": { "description": "工作台聚合数据", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ScheduleBoard" } } } },
                        "400": error_response("from/to 缺失、格式错误或日期跨度非法"),
                        "403": error_response("缺少 schedule:read")
                    }
                }
            },
            "/admin/schedules/ensure": {
                "post": {
                    "tags": ["Schedules"],
                    "summary": "幂等地为一批房间补默认课表（已有交集课表归入 existing）",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/EnsureSchedulesRequest" } } } },
                    "responses": {
                        "200": { "description": "新建 / 已存在的课表", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/EnsureSchedulesResponse" } } } },
                        "400": error_response("room_ids 为空或超限、日期区间非法、房间已停用"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("房间不存在")
                    }
                }
            },
            "/admin/schedules/{id}": {
                "get": {
                    "tags": ["Schedules"],
                    "summary": "课表详情",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "课表", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Schedule" } } } },
                        "404": error_response("课表不存在")
                    }
                },
                "patch": {
                    "tags": ["Schedules"],
                    "summary": "局部更新课表；成功后重建未来排课实例",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateScheduleRequest" } } } },
                    "responses": {
                        "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Schedule" } } } },
                        "400": error_response("参数校验失败 / 没有需要更新的字段"),
                        "404": error_response("课表不存在"),
                        "500": error_response("课表已更新但重建排课实例失败；可调用 rebuild 接口重试")
                    }
                },
                "delete": {
                    "tags": ["Schedules"],
                    "summary": "删除课表并级联删除实例",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "404": error_response("课表不存在")
                    }
                }
            },
            "/admin/schedules/{id}/weekly-slots": {
                "get": {
                    "tags": ["Schedules"],
                    "summary": "周模板",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "周模板", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/WeeklySlot" } } } } },
                        "404": error_response("课表不存在")
                    }
                },
                "put": {
                    "tags": ["Schedules"],
                    "summary": "全量替换周模板；成功后重建未来排课实例",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ReplaceWeeklySlotsRequest" } } } },
                    "responses": {
                        "200": { "description": "替换后的周模板", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/WeeklySlot" } } } } },
                        "400": error_response("时段校验失败"),
                        "404": error_response("课表不存在"),
                        "500": error_response("已写入但重建排课实例失败")
                    }
                }
            },
            "/admin/schedules/{id}/exceptions": {
                "get": {
                    "tags": ["Schedules"],
                    "summary": "单日覆盖列表",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "from", "in": "query", "schema": { "type": "string", "format": "date" } },
                        { "name": "to", "in": "query", "schema": { "type": "string", "format": "date" } }
                    ],
                    "responses": {
                        "200": { "description": "单日覆盖", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/ScheduleException" } } } } },
                        "404": error_response("课表不存在")
                    }
                }
            },
            "/admin/schedules/{id}/exceptions/{date}": {
                "put": {
                    "tags": ["Schedules"],
                    "summary": "upsert 单日覆盖（cancel / replace）；成功后重建未来排课实例",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "date", "in": "path", "required": true, "schema": { "type": "string", "format": "date" } }
                    ],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpsertExceptionRequest" } } } },
                    "responses": {
                        "200": { "description": "覆盖已保存", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ScheduleException" } } } },
                        "400": error_response("时段校验失败"),
                        "404": error_response("课表不存在"),
                        "500": error_response("已写入但重建排课实例失败")
                    }
                },
                "delete": {
                    "tags": ["Schedules"],
                    "summary": "删除单日覆盖并重建未来排课实例",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "date", "in": "path", "required": true, "schema": { "type": "string", "format": "date" } }
                    ],
                    "responses": {
                        "204": { "description": "已删除" },
                        "404": error_response("单日覆盖不存在"),
                        "500": error_response("已删除但重建排课实例失败")
                    }
                }
            },
            "/admin/schedules/{id}/occurrences": {
                "get": {
                    "tags": ["Schedules"],
                    "summary": "排课实例（UTC start_at/end_at + 区域时区 start_local/end_local；默认按区域时区今天起 7 天）",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "from", "in": "query", "schema": { "type": "string", "format": "date" } },
                        { "name": "to", "in": "query", "schema": { "type": "string", "format": "date" } }
                    ],
                    "responses": {
                        "200": { "description": "排课实例", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/Occurrence" } } } } },
                        "400": error_response("日期区间不合法"),
                        "404": error_response("课表不存在")
                    }
                }
            },
            "/admin/schedules/{id}/rebuild": {
                "post": {
                    "tags": ["Schedules"],
                    "summary": "手动重建未来排课实例",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "重建结果", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RebuildResult" } } } },
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("课表不存在"),
                        "500": error_response("重建失败")
                    }
                }
            },
            "/admin/schedule-events": {
                "get": {
                    "tags": ["ScheduleEvents"],
                    "summary": "事件分页（triggered_at 倒序；含 timezone / triggered_at_local）",
                    "security": secured(),
                    "parameters": [
                        { "name": "room_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "kind", "in": "query", "schema": { "type": "string", "enum": ["occurrence_start", "occurrence_end", "occurrence_start_before", "daily_time", "room_occupied", "room_idle"] } },
                        { "name": "from", "in": "query", "schema": { "type": "string", "format": "date-time" } },
                        { "name": "to", "in": "query", "schema": { "type": "string", "format": "date-time" } },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } }
                    ],
                    "responses": {
                        "200": { "description": "事件分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ScheduleEventPage" } } } },
                        "400": error_response("事件类型或分页参数不合法"),
                        "403": error_response("缺少 schedule:event:read")
                    }
                }
            },
            "/admin/schedule-events/{id}": {
                "get": {
                    "tags": ["ScheduleEvents"],
                    "summary": "事件 + 投递明细",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "事件与投递", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ScheduleEventDetail" } } } },
                        "404": error_response("事件不存在")
                    }
                }
            },
            "/admin/schedule-events/{id}/replay": {
                "post": {
                    "tags": ["ScheduleEvents"],
                    "summary": "把事件全部投递重置为 pending",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "重放结果", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ReplayResult" } } } },
                        "403": error_response("缺少 schedule:event:manage"),
                        "404": error_response("事件不存在")
                    }
                }
            },
            "/admin/schedule-events/{id}/deliveries/{delivery_id}/retry": {
                "post": {
                    "tags": ["ScheduleEvents"],
                    "summary": "单条投递立即重试",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "delivery_id", "in": "path", "required": true, "schema": { "type": "integer" } }
                    ],
                    "responses": {
                        "204": { "description": "已重置为待投递" },
                        "403": error_response("缺少 schedule:event:manage"),
                        "404": error_response("事件或投递记录不存在")
                    }
                }
            },
            "/admin/storage/objects/presign": {
                "post": {
                    "tags": ["Storage"],
                    "summary": "初始化对象上传：返回 objects/{sha256} 的短 TTL 预签名 PUT",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/StorageObjectRequest" } } } },
                    "responses": {
                        "200": { "description": "预签名上传地址", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/PresignObjectReply" } } } },
                        "400": error_response("sha256 / size / content_type 不合法"),
                        "403": error_response("缺少 storage:manage"),
                        "503": error_response("未配置 S3 或对象存储不可用")
                    }
                }
            },
            "/admin/storage/objects": {
                "post": {
                    "tags": ["Storage"],
                    "summary": "登记已上传对象（服务端从 S3 读回并重算 sha256/size，不一致 400）",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/StorageObjectRequest" } } } },
                    "responses": {
                        "201": { "description": "已登记", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/StorageObject" } } } },
                        "400": error_response("sha256 不匹配 / size 不一致 / 参数不合法"),
                        "403": error_response("缺少 storage:manage"),
                        "503": error_response("未配置 S3 或读取对象失败")
                    }
                }
            },
            "/admin/storage/objects/{sha256}": {
                "get": {
                    "tags": ["Storage"],
                    "summary": "对象详情 + 预签名下载地址",
                    "security": secured(),
                    "parameters": [{ "name": "sha256", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": {
                        "200": { "description": "对象详情", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/StorageObjectReply" } } } },
                        "400": error_response("sha256 不合法"),
                        "403": error_response("缺少 storage:read"),
                        "404": error_response("对象未登记"),
                        "503": error_response("未配置 S3 或对象存储不可用")
                    }
                }
            },
            "/admin/images": {
                "get": {
                    "tags": ["Images"],
                    "summary": "镜像分页（kind / status / name）",
                    "security": secured(),
                    "parameters": [
                        { "name": "kind", "in": "query", "schema": { "$ref": "#/components/schemas/ImageKind" } },
                        { "name": "status", "in": "query", "schema": { "$ref": "#/components/schemas/ArtifactStatus" } },
                        { "name": "name", "in": "query", "schema": { "type": "string" }, "description": "按名称模糊匹配。" },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } }
                    ],
                    "responses": {
                        "200": { "description": "镜像分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ImagePage" } } } },
                        "400": error_response("kind / status / 分页参数不合法"),
                        "403": error_response("缺少 image:read")
                    }
                },
                "post": {
                    "tags": ["Images"],
                    "summary": "创建镜像制品（uploaded；name+version+platform 唯一）",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateImageRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ImageArtifact" } } } },
                        "400": error_response("名称/版本/平台不合法，kind 与 format 不匹配，或 compression 非 none"),
                        "403": error_response("缺少 image:manage"),
                        "409": error_response("同名同版本同平台的镜像已存在")
                    }
                }
            },
            "/admin/images/{id}": {
                "get": {
                    "tags": ["Images"],
                    "summary": "镜像详情 + 分片 + 最近验证报告 + 已发布通道",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "镜像详情", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ImageDetail" } } } },
                        "403": error_response("缺少 image:read"),
                        "404": error_response("镜像不存在")
                    }
                }
            },
            "/admin/images/{id}/manifest": {
                "get": {
                    "tags": ["Images"],
                    "summary": "读取 manifest JSON + 预签名原始下载地址",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "manifest 与下载地址", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ManifestReply" } } } },
                        "403": error_response("缺少 image:read"),
                        "404": error_response("镜像不存在"),
                        "409": error_response("镜像尚未提交 manifest"),
                        "503": error_response("未配置 S3 或对象存储不可用")
                    }
                },
                "post": {
                    "tags": ["Images"],
                    "summary": "提交 manifest：校验分片对象、签名并写入 S3，状态置 manifest_ready",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SubmitManifestRequest" } } } },
                    "responses": {
                        "200": { "description": "manifest 已生成", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SubmitManifestReply" } } } },
                        "400": error_response("parts 不连续 / 分片对象未登记 / size 不一致 / compression 非 none"),
                        "403": error_response("缺少 image:manage"),
                        "404": error_response("镜像不存在"),
                        "409": error_response("已发布/废弃/吊销的镜像不能提交 manifest"),
                        "503": error_response("未配置 S3 或写入失败")
                    }
                }
            },
            "/admin/images/{id}/verify": {
                "post": {
                    "tags": ["Images"],
                    "summary": "同步验证：分片 sha256/size、总大小、格式 magic、manifest 签名；写验证报告",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "验证结果（artifact.status = verified，失败时回到 manifest_ready 且 report.ok=false）", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/VerifyReply" } } } },
                        "400": error_response("compression 非 none"),
                        "403": error_response("缺少 image:manage"),
                        "404": error_response("镜像不存在"),
                        "409": error_response("镜像尚未提交 manifest 或没有分片"),
                        "503": error_response("未配置 S3 或对象存储不可用")
                    }
                }
            },
            "/admin/images/{id}/publish": {
                "post": {
                    "tags": ["Images"],
                    "summary": "发布到通道（dev / test / prod），仅 verified 可发布",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/PublishImageRequest" } } } },
                    "responses": {
                        "200": { "description": "已发布", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ImageArtifact" } } } },
                        "400": error_response("channel 不合法"),
                        "403": error_response("缺少 image:manage"),
                        "404": error_response("镜像不存在"),
                        "409": error_response("镜像不是 verified 状态")
                    }
                }
            },
            "/admin/rooms/{id}/images/{image_id}": {
                "put": {
                    "tags": ["Images"],
                    "summary": "把已发布镜像分配到房间（创建 download transfer）",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "image_id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }
                    ],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AssignImageRequest" } } } },
                    "responses": {
                        "200": { "description": "分配结果", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ImageAssignment" } } } },
                        "400": error_response("channel 不合法"),
                        "403": error_response("缺少 image:manage"),
                        "404": error_response("镜像或房间不存在"),
                        "409": error_response("镜像未发布")
                    }
                },
                "delete": {
                    "tags": ["Images"],
                    "summary": "取消房间分配",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "image_id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }
                    ],
                    "responses": {
                        "204": { "description": "已取消分配" },
                        "403": error_response("缺少 image:manage"),
                        "404": error_response("该房间未分配此镜像")
                    }
                }
            },
            "/admin/transfers": {
                "get": {
                    "tags": ["Storage"],
                    "summary": "分发/收集任务分页（按房间过滤，created_at 倒序）",
                    "security": secured(),
                    "parameters": [
                        { "name": "room_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } }
                    ],
                    "responses": {
                        "200": { "description": "任务分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TransferPage" } } } },
                        "403": error_response("缺少 storage:read")
                    }
                }
            },
            "/admin/transfers/{id}/retry": {
                "post": {
                    "tags": ["Storage"],
                    "summary": "重试分发任务（failed / expired → pending，进度清零）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "重置后的任务", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Transfer" } } } },
                        "403": error_response("缺少 storage:manage"),
                        "404": error_response("任务不存在"),
                        "409": error_response("任务不是 failed / expired 状态")
                    }
                }
            },
            "/admin/collections": {
                "get": {
                    "tags": ["Storage"],
                    "summary": "收集登记分页（按房间过滤，created_at 倒序）",
                    "security": secured(),
                    "parameters": [
                        { "name": "room_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } }
                    ],
                    "responses": {
                        "200": { "description": "收集分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CollectionPage" } } } },
                        "403": error_response("缺少 storage:read")
                    }
                }
            },
            "/admin/agent-configs": {
                "get": {
                    "tags": ["AgentConfigs"],
                    "summary": "所有作用域的节点配置（global / area / room）",
                    "security": secured(),
                    "responses": {
                        "200": { "description": "配置列表", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AgentConfigList" } } } },
                        "403": error_response("缺少 device:read")
                    }
                },
                "put": {
                    "tags": ["AgentConfigs"],
                    "summary": "写入作用域配置（upsert 后审计并二次下发 config.update）",
                    "description": "global 的 scope_id 必须为空；area/room 必须提供且目标存在。policy 为局部 patch，先做范围校验。",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateAgentConfigRequest" } } } },
                    "responses": {
                        "200": { "description": "写入结果与成功下发的房间数", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateAgentConfigReply" } } } },
                        "400": error_response("scope 与 scope_id 不匹配，或 policy 超出范围"),
                        "403": error_response("缺少 device:manage"),
                        "404": error_response("区域或房间不存在")
                    }
                }
            },
            "/admin/agent-configs/effective": {
                "get": {
                    "tags": ["AgentConfigs"],
                    "summary": "某房间合并后的生效策略（global → area → room）",
                    "security": secured(),
                    "parameters": [
                        { "name": "room_id", "in": "query", "required": true, "schema": { "type": "string", "format": "uuid" } }
                    ],
                    "responses": {
                        "200": { "description": "生效策略与最大 revision（无配置为 0）", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/EffectiveAgentPolicyReply" } } } },
                        "403": error_response("缺少 device:read"),
                        "404": error_response("房间不存在")
                    }
                }
            },
            "/admin/device-events": {
                "get": {
                    "tags": ["DeviceEvents"],
                    "summary": "设备在线事件（id 倒序；limit 1–200，默认 50）",
                    "security": secured(),
                    "parameters": [
                        { "name": "limit", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 } },
                        { "name": "after_id", "in": "query", "schema": { "type": "integer", "format": "int64", "minimum": 0 }, "description": "只返回 id > after_id 的事件。" }
                    ],
                    "responses": {
                        "200": { "description": "事件列表", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/PresenceEventList" } } } },
                        "400": error_response("limit / after_id 不合法"),
                        "403": error_response("缺少 device:read")
                    }
                }
            },
            "/admin/device-events/stream": {
                "get": {
                    "tags": ["DeviceEvents"],
                    "summary": "设备在线事件 SSE 流（先补发历史，之后每 2s 增量 + 15s 心跳）",
                    "description": "事件类型 `device.presence`，data 为 PresenceEvent JSON；心跳事件类型 `heartbeat`。跨副本以查库为准。",
                    "security": secured(),
                    "parameters": [
                        { "name": "after_id", "in": "query", "schema": { "type": "integer", "format": "int64", "minimum": 0 }, "description": "从该事件 id 之后开始补发；缺省返回最近 200 条。" }
                    ],
                    "responses": {
                        "200": { "description": "SSE 流", "content": { "text/event-stream": { "schema": { "type": "string" } } } },
                        "400": error_response("after_id 不合法"),
                        "403": error_response("缺少 device:read")
                    }
                }
            },
            "/admin/device-presence-deliveries": {
                "get": {
                    "tags": ["DeviceEvents"],
                    "summary": "设备在线事件投递记录（id 倒序，排障用）",
                    "security": secured(),
                    "parameters": [
                        { "name": "status", "in": "query", "schema": { "type": "string", "enum": ["pending", "delivered", "failed"] } },
                        { "name": "limit", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 200, "default": 100 } }
                    ],
                    "responses": {
                        "200": { "description": "投递列表", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/PresenceDeliveryList" } } } },
                        "400": error_response("status / limit 不合法"),
                        "403": error_response("缺少 device:read")
                    }
                }
            },
            "/machine/report": {
                "post": {
                    "tags": ["MachineAgent"],
                    "summary": "room-agent 上报机器快照（fleet Bearer token）",
                    "description": "认证头：Authorization: Bearer <MACHINE_AGENT_TOKEN>；服务端未配置 token 时返回 503。按 device_slot 解析已注册设备（issued/active），同一 (serial, collected_at) 幂等。",
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/MachineReportRequest" } } } },
                    "responses": {
                        "202": { "description": "已接收", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/MachineReportReply" } } } },
                        "400": error_response("上报体校验失败"),
                        "401": error_response("机器上报 token 无效"),
                        "404": error_response("device_slot 对应的设备不存在"),
                        "503": error_response("机器上报未启用")
                    }
                }
            },
            "/admin/machines": {
                "get": {
                    "tags": ["Machines"],
                    "summary": "机器分页（设备 + 房间/区域 + 最新快照摘要）",
                    "security": secured(),
                    "parameters": [
                        { "name": "area_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "room_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string" }, "description": "serial / 槽位 / 房间 code/name / hostname 模糊匹配" },
                        { "name": "health", "in": "query", "schema": { "type": "string", "enum": ["ok", "warning", "critical", "missing", "unknown"] }, "description": "missing = 从未上报" },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } }
                    ],
                    "responses": {
                        "200": { "description": "机器分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/MachinePage" } } } },
                        "400": error_response("参数不合法"),
                        "403": error_response("缺少 device:read")
                    }
                }
            },
            "/admin/machines/stream": {
                "get": {
                    "tags": ["Machines"],
                    "summary": "机器快照 SSE 实时流（新快照 2s 内推送）",
                    "description": "首次连接从当前最新快照 id 开始（不补历史）；断线重连时带最后收到的 id 作为 after_id，可补发最多 200 条。帧事件：`ready`（建连即发，data 为空）、`machine.snapshot`（data 为 MachineSnapshotPoint）、每 15s `heartbeat`。浏览器 EventSource 不能带 Authorization，请用 fetch + ReadableStream。",
                    "security": secured(),
                    "parameters": [
                        { "name": "after_id", "in": "query", "schema": { "type": "integer", "format": "int64", "minimum": 0 }, "description": "从该快照 id 之后补发；缺省 = 只推送连接之后的新快照。" },
                        { "name": "serial", "in": "query", "schema": { "type": "string" }, "description": "只订阅某台机器；缺省为全部机器。" }
                    ],
                    "responses": {
                        "200": { "description": "SSE 流", "content": { "text/event-stream": { "schema": { "type": "string" } } } },
                        "400": error_response("after_id 不合法"),
                        "403": error_response("缺少 device:read")
                    }
                }
            },
            "/admin/machines/{serial}": {
                "get": {
                    "tags": ["Machines"],
                    "summary": "机器详情（设备 + 最新完整快照 + 历史）",
                    "security": secured(),
                    "parameters": [
                        { "name": "serial", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "history_limit", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 } }
                    ],
                    "responses": {
                        "200": { "description": "机器详情", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/MachineDetail" } } } },
                        "403": error_response("缺少 device:read"),
                        "404": error_response("机器不存在")
                    }
                }
            },
            "/admin/machines/{serial}/snapshots": {
                "get": {
                    "tags": ["Machines"],
                    "summary": "机器快照历史（元数据，collected_at 倒序）",
                    "security": secured(),
                    "parameters": [
                        { "name": "serial", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "name": "limit", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 } }
                    ],
                    "responses": {
                        "200": { "description": "快照历史", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/MachineSnapshotPoint" } } } } },
                        "403": error_response("缺少 device:read"),
                        "404": error_response("机器不存在")
                    }
                }
            },
            "/admin/tickets": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "工单分页（updated_at 倒序）",
                    "security": secured(),
                    "parameters": [
                        { "name": "status", "in": "query", "schema": { "$ref": "#/components/schemas/TicketStatus" } },
                        { "name": "category", "in": "query", "schema": { "$ref": "#/components/schemas/TicketCategory" } },
                        { "name": "priority", "in": "query", "schema": { "$ref": "#/components/schemas/TicketPriority" } },
                        { "name": "source", "in": "query", "schema": { "$ref": "#/components/schemas/TicketSource" } },
                        { "name": "assignee_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "unassigned", "in": "query", "schema": { "type": "boolean" }, "description": "true = 只看未分配工单；与 assignee_id 同时出现时以 assignee_id 为准。" },
                        { "name": "mine", "in": "query", "schema": { "type": "boolean" }, "description": "true = 只看 assignee_id = 当前用户的工单；是否只看未结束由 active 参数控制。" },
                        { "name": "active", "in": "query", "schema": { "type": "boolean" }, "description": "true = 只看 open/in_progress。" },
                        { "name": "overdue", "in": "query", "schema": { "type": "boolean" }, "description": "true = 只看超时未结束工单。" },
                        { "name": "area_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "room_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "serial", "in": "query", "schema": { "type": "string" } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string" }, "description": "标题/描述/单号模糊匹配（≤100 字符）。" },
                        { "name": "from", "in": "query", "schema": { "type": "string" }, "description": "创建时间下限，RFC3339 或 YYYY-MM-DD。" },
                        { "name": "to", "in": "query", "schema": { "type": "string" }, "description": "创建时间上限，RFC3339 或 YYYY-MM-DD（日期含当天）。" },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 } }
                    ],
                    "responses": {
                        "200": { "description": "工单分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketPage" } } } },
                        "400": error_response("参数不合法"),
                        "403": error_response("缺少 ticket:read")
                    }
                },
                "post": {
                    "tags": ["Tickets"],
                    "summary": "人工创建工单",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateTicketRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Ticket" } } } },
                        "400": error_response("标题/描述/设备槽位不合法"),
                        "403": error_response("缺少 ticket:create"),
                        "404": error_response("房间或区域不存在")
                    }
                }
            },
            "/admin/tickets/summary": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "轻量计数：状态分布、未分配、超时、我的待办",
                    "security": secured(),
                    "responses": {
                        "200": { "description": "工单计数", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketSummary" } } } },
                        "403": error_response("缺少 ticket:read")
                    }
                }
            },
            "/admin/tickets/stats": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "历史分析聚合（缺省近 30 天）",
                    "description": "MTTR = 区间内 resolved_at - created_at 的平均值（排除 cancelled），同时返回中位数；按天趋势按关联区域时区聚合本地日期。",
                    "security": secured(),
                    "parameters": [
                        { "name": "from", "in": "query", "schema": { "type": "string" }, "description": "RFC3339 或 YYYY-MM-DD，默认 now-30d。" },
                        { "name": "to", "in": "query", "schema": { "type": "string" }, "description": "RFC3339 或 YYYY-MM-DD，默认 now。" }
                    ],
                    "responses": {
                        "200": { "description": "统计聚合", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketStats" } } } },
                        "400": error_response("时间参数不合法"),
                        "403": error_response("缺少 ticket:read")
                    }
                }
            },
            "/admin/tickets/assignees": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "可转派人员（拥有 ticket:handle 的启用用户）",
                    "security": secured(),
                    "responses": {
                        "200": { "description": "人员列表", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/TicketAssignee" } } } } },
                        "403": error_response("缺少 ticket:handle")
                    }
                }
            },
            "/admin/tickets/{id}": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "工单详情（含时间线与关联上下文）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "工单详情", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketDetail" } } } },
                        "403": error_response("缺少 ticket:read"),
                        "404": error_response("工单不存在")
                    }
                },
                "patch": {
                    "tags": ["Tickets"],
                    "summary": "修改标题/描述/优先级/分类（仅未结束工单）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateTicketRequest" } } } },
                    "responses": {
                        "200": { "description": "更新后的工单", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Ticket" } } } },
                        "400": error_response("标题/描述不合法"),
                        "403": error_response("缺少 ticket:manage"),
                        "404": error_response("工单不存在"),
                        "409": error_response("工单已结束，不能修改")
                    }
                }
            },
            "/admin/tickets/{id}/claim": {
                "post": {
                    "tags": ["Tickets"],
                    "summary": "认领（open → in_progress，幂等）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "认领后的工单", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Ticket" } } } },
                        "403": error_response("缺少 ticket:handle"),
                        "404": error_response("工单不存在"),
                        "409": error_response("工单已被他人认领或状态不允许认领")
                    }
                }
            },
            "/admin/tickets/{id}/assign": {
                "post": {
                    "tags": ["Tickets"],
                    "summary": "转派（assignee_id = null 表示释放认领）",
                    "description": "请求体必填：assignee_id 缺失 → 400；显式 null 表示释放认领。目标用户必须拥有 ticket:handle 权限；已结束工单不能转派。",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AssignTicketRequest" } } } },
                    "responses": {
                        "200": { "description": "转派后的工单", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Ticket" } } } },
                        "400": error_response("目标处理人没有 ticket:handle 权限"),
                        "403": error_response("缺少 ticket:manage"),
                        "404": error_response("工单不存在"),
                        "409": error_response("工单已结束，不能转派")
                    }
                }
            },
            "/admin/tickets/{id}/transition": {
                "post": {
                    "tags": ["Tickets"],
                    "summary": "状态流转（服务端状态机校验）",
                    "description": "允许：open→in_progress/cancelled、in_progress→resolved/cancelled、resolved→closed/open、closed→open。resolved 必须带 resolution。非法流转 → 409。",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TransitionTicketRequest" } } } },
                    "responses": {
                        "200": { "description": "流转后的工单", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Ticket" } } } },
                        "400": error_response("标记解决但缺少 resolution"),
                        "403": error_response("缺少 ticket:handle"),
                        "404": error_response("工单不存在"),
                        "409": error_response("状态机不允许该流转")
                    }
                }
            },
            "/admin/tickets/{id}/comments": {
                "post": {
                    "tags": ["Tickets"],
                    "summary": "追加评论（写入时间线）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CommentTicketRequest" } } } },
                    "responses": {
                        "201": { "description": "新建的评论事件", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketEvent" } } } },
                        "400": error_response("评论内容为空或过长"),
                        "403": error_response("缺少 ticket:handle"),
                        "404": error_response("工单不存在")
                    }
                }
            },
            "/admin/tickets/export": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "导出工单 CSV（同列表筛选，最多 5000 行）",
                    "security": secured(),
                    "parameters": [
                        { "name": "status", "in": "query", "schema": { "$ref": "#/components/schemas/TicketStatus" } },
                        { "name": "category", "in": "query", "schema": { "$ref": "#/components/schemas/TicketCategory" } },
                        { "name": "priority", "in": "query", "schema": { "$ref": "#/components/schemas/TicketPriority" } },
                        { "name": "source", "in": "query", "schema": { "$ref": "#/components/schemas/TicketSource" } },
                        { "name": "assignee_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "unassigned", "in": "query", "schema": { "type": "boolean" } },
                        { "name": "mine", "in": "query", "schema": { "type": "boolean" } },
                        { "name": "active", "in": "query", "schema": { "type": "boolean" } },
                        { "name": "overdue", "in": "query", "schema": { "type": "boolean" } },
                        { "name": "area_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "room_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "serial", "in": "query", "schema": { "type": "string" } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string" } },
                        { "name": "from", "in": "query", "schema": { "type": "string" } },
                        { "name": "to", "in": "query", "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": { "description": "CSV 文件（UTF-8 BOM，中文表头）", "content": { "text/csv": { "schema": { "type": "string" } } } },
                        "403": error_response("缺少 ticket:export 或 ticket:read")
                    }
                }
            },
            "/admin/tickets/stream": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "工单事件 SSE（先补发 after_event_id 之后，再每 2s 增量）",
                    "security": secured(),
                    "parameters": [
                        { "name": "after_event_id", "in": "query", "schema": { "type": "integer", "format": "int64", "minimum": 0 } }
                    ],
                    "responses": {
                        "200": { "description": "SSE：ticket.event / heartbeat", "content": { "text/event-stream": { "schema": { "type": "string" } } } },
                        "403": error_response("缺少 ticket:read")
                    }
                }
            },
            "/admin/ticket-rules": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "自动建单规则列表",
                    "security": secured(),
                    "responses": {
                        "200": { "description": "规则列表", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketRuleList" } } } },
                        "403": error_response("缺少 ticket:rule:manage 或 ticket:manage")
                    }
                },
                "post": {
                    "tags": ["Tickets"],
                    "summary": "新建自动建单规则",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateTicketRuleRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketRule" } } } },
                        "400": error_response("名称/阈值/范围不合法"),
                        "403": error_response("缺少 ticket:rule:manage 或 ticket:manage")
                    }
                }
            },
            "/admin/ticket-rules/{id}": {
                "patch": {
                    "tags": ["Tickets"],
                    "summary": "部分更新自动建单规则（signal_type 不可改）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateTicketRuleRequest" } } } },
                    "responses": {
                        "200": { "description": "更新后的规则", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketRule" } } } },
                        "400": error_response("名称/阈值/范围不合法"),
                        "403": error_response("缺少 ticket:rule:manage 或 ticket:manage"),
                        "404": error_response("规则不存在")
                    }
                },
                "delete": {
                    "tags": ["Tickets"],
                    "summary": "删除自动建单规则",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "403": error_response("缺少 ticket:rule:manage 或 ticket:manage"),
                        "404": error_response("规则不存在")
                    }
                }
            },
            "/admin/terms": {
                "get": {
                    "tags": ["Terms"],
                    "summary": "学期分页",
                    "security": secured(),
                    "parameters": [
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string", "maxLength": 100 }, "description": "按 code / name 模糊搜索。" },
                        { "name": "include_inactive", "in": "query", "schema": { "type": "boolean" }, "description": "是否包含停用记录，缺省 false。" }
                    ],
                    "responses": {
                        "200": { "description": "学期分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TermPage" } } } },
                        "403": error_response("缺少 schedule:read")
                    }
                },
                "post": {
                    "tags": ["Terms"],
                    "summary": "创建学期",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateTermRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TermView" } } } },
                        "400": error_response("参数校验失败"),
                        "403": error_response("缺少 schedule:manage"),
                        "409": error_response("编码已存在")
                    }
                }
            },
            "/admin/terms/{id}": {
                "get": {
                    "tags": ["Terms"],
                    "summary": "学期详情",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "学期", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TermView" } } } },
                        "403": error_response("缺少 schedule:read"),
                        "404": error_response("学期不存在")
                    }
                },
                "patch": {
                    "tags": ["Terms"],
                    "summary": "局部更新学期",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateTermRequest" } } } },
                    "responses": {
                        "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TermView" } } } },
                        "400": error_response("参数校验失败 / 没有需要更新的字段"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("学期不存在"),
                        "409": error_response("编码已存在")
                    }
                },
                "delete": {
                    "tags": ["Terms"],
                    "summary": "删除学期",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("学期不存在"),
                        "409": error_response("学期仍被排课计划引用")
                    }
                }
            },
            "/admin/teachers": {
                "get": {
                    "tags": ["Teachers"],
                    "summary": "教师分页",
                    "security": secured(),
                    "parameters": [
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string", "maxLength": 100 }, "description": "按 code / name 模糊搜索。" },
                        { "name": "include_inactive", "in": "query", "schema": { "type": "boolean" }, "description": "是否包含停用记录，缺省 false。" }
                    ],
                    "responses": {
                        "200": { "description": "教师分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TeacherPage" } } } },
                        "403": error_response("缺少 schedule:read")
                    }
                },
                "post": {
                    "tags": ["Teachers"],
                    "summary": "创建教师",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateTeacherRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TeacherView" } } } },
                        "400": error_response("参数校验失败"),
                        "403": error_response("缺少 schedule:manage"),
                        "409": error_response("编码已存在")
                    }
                }
            },
            "/admin/teachers/{id}": {
                "get": {
                    "tags": ["Teachers"],
                    "summary": "教师详情",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "教师", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TeacherView" } } } },
                        "403": error_response("缺少 schedule:read"),
                        "404": error_response("教师不存在")
                    }
                },
                "patch": {
                    "tags": ["Teachers"],
                    "summary": "局部更新教师",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateTeacherRequest" } } } },
                    "responses": {
                        "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TeacherView" } } } },
                        "400": error_response("参数校验失败 / 没有需要更新的字段"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("教师不存在"),
                        "409": error_response("编码已存在")
                    }
                },
                "delete": {
                    "tags": ["Teachers"],
                    "summary": "删除教师",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("教师不存在"),
                        "409": error_response("教师仍被计划条目引用")
                    }
                }
            },
            "/admin/classes": {
                "get": {
                    "tags": ["Classes"],
                    "summary": "班级分页",
                    "security": secured(),
                    "parameters": [
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string", "maxLength": 100 }, "description": "按 code / name 模糊搜索。" },
                        { "name": "include_inactive", "in": "query", "schema": { "type": "boolean" }, "description": "是否包含停用记录，缺省 false。" }
                    ],
                    "responses": {
                        "200": { "description": "班级分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ClassPage" } } } },
                        "403": error_response("缺少 schedule:read")
                    }
                },
                "post": {
                    "tags": ["Classes"],
                    "summary": "创建班级",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateClassRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ClassView" } } } },
                        "400": error_response("参数校验失败"),
                        "403": error_response("缺少 schedule:manage"),
                        "409": error_response("编码已存在")
                    }
                }
            },
            "/admin/classes/{id}": {
                "get": {
                    "tags": ["Classes"],
                    "summary": "班级详情",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "班级", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ClassView" } } } },
                        "403": error_response("缺少 schedule:read"),
                        "404": error_response("班级不存在")
                    }
                },
                "patch": {
                    "tags": ["Classes"],
                    "summary": "局部更新班级",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateClassRequest" } } } },
                    "responses": {
                        "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ClassView" } } } },
                        "400": error_response("参数校验失败 / 没有需要更新的字段"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("班级不存在"),
                        "409": error_response("编码已存在")
                    }
                },
                "delete": {
                    "tags": ["Classes"],
                    "summary": "删除班级",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("班级不存在"),
                        "409": error_response("班级仍被计划条目引用")
                    }
                }
            },
            "/admin/courses": {
                "get": {
                    "tags": ["Courses"],
                    "summary": "课程分页",
                    "security": secured(),
                    "parameters": [
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } },
                        { "name": "keyword", "in": "query", "schema": { "type": "string", "maxLength": 100 }, "description": "按 code / name 模糊搜索。" },
                        { "name": "include_inactive", "in": "query", "schema": { "type": "boolean" }, "description": "是否包含停用记录，缺省 false。" }
                    ],
                    "responses": {
                        "200": { "description": "课程分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CoursePage" } } } },
                        "403": error_response("缺少 schedule:read")
                    }
                },
                "post": {
                    "tags": ["Courses"],
                    "summary": "创建课程",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateCourseRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CourseView" } } } },
                        "400": error_response("参数校验失败"),
                        "403": error_response("缺少 schedule:manage"),
                        "409": error_response("编码已存在")
                    }
                }
            },
            "/admin/courses/{id}": {
                "get": {
                    "tags": ["Courses"],
                    "summary": "课程详情",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "课程", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CourseView" } } } },
                        "403": error_response("缺少 schedule:read"),
                        "404": error_response("课程不存在")
                    }
                },
                "patch": {
                    "tags": ["Courses"],
                    "summary": "局部更新课程",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateCourseRequest" } } } },
                    "responses": {
                        "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CourseView" } } } },
                        "400": error_response("参数校验失败 / 没有需要更新的字段"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("课程不存在"),
                        "409": error_response("编码已存在")
                    }
                },
                "delete": {
                    "tags": ["Courses"],
                    "summary": "删除课程",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("课程不存在"),
                        "409": error_response("课程仍被计划条目引用")
                    }
                }
            },
            "/admin/schedule-plans": {
                "get": {
                    "tags": ["SchedulePlans"],
                    "summary": "排课计划分页（可按学期 / 状态过滤）",
                    "security": secured(),
                    "parameters": [
                        { "name": "term_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "status", "in": "query", "schema": { "type": "string", "enum": ["draft", "published"] }, "description": "计划状态；非法值 400。" },
                        { "name": "page", "in": "query", "schema": { "type": "integer", "minimum": 1 } },
                        { "name": "page_size", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100 } }
                    ],
                    "responses": {
                        "200": { "description": "排课计划分页", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SchedulePlanPage" } } } },
                        "400": error_response("状态或分页参数非法"),
                        "403": error_response("缺少 schedule:read")
                    }
                },
                "post": {
                    "tags": ["SchedulePlans"],
                    "summary": "创建草稿计划（version = 1）",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateSchedulePlanRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建草稿计划", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SchedulePlan" } } } },
                        "400": error_response("参数校验失败 / 学期已停用"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("学期不存在")
                    }
                }
            },
            "/admin/schedule-plans/{id}": {
                "get": {
                    "tags": ["SchedulePlans"],
                    "summary": "排课计划详情（计划 + 条目，含名称快照）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "计划与条目", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/PlanDetailResponse" } } } },
                        "403": error_response("缺少 schedule:read"),
                        "404": error_response("排课计划不存在")
                    }
                },
                "patch": {
                    "tags": ["SchedulePlans"],
                    "summary": "重命名计划（仅草稿；已发布 409）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateSchedulePlanRequest" } } } },
                    "responses": {
                        "200": { "description": "已更新", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SchedulePlan" } } } },
                        "400": error_response("参数校验失败"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("排课计划不存在"),
                        "409": error_response("已发布的计划不可修改")
                    }
                },
                "delete": {
                    "tags": ["SchedulePlans"],
                    "summary": "删除计划（仅草稿；已发布 409；条目级联删除）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("排课计划不存在"),
                        "409": error_response("已发布的计划不可删除")
                    }
                }
            },
            "/admin/schedule-plans/{id}/clone": {
                "post": {
                    "tags": ["SchedulePlans"],
                    "summary": "克隆为新草稿（任意状态可克隆；version = 同学期最大值 + 1，名称加「副本」）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "201": { "description": "新草稿计划", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SchedulePlan" } } } },
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("排课计划不存在")
                    }
                }
            },
            "/admin/schedule-plans/{id}/entries": {
                "get": {
                    "tags": ["SchedulePlans"],
                    "summary": "计划条目列表（按房间 → 星期 → 开始时间排序）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "条目列表", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/PlanEntry" } } } } },
                        "403": error_response("缺少 schedule:read"),
                        "404": error_response("排课计划不存在")
                    }
                },
                "put": {
                    "tags": ["SchedulePlans"],
                    "summary": "全量替换计划条目（仅草稿；同房间同日时段不得重叠）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ReplacePlanEntriesRequest" } } } },
                    "responses": {
                        "200": { "description": "替换后的条目", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/PlanEntry" } } } } },
                        "400": error_response("参数校验失败 / 时段重叠 / 引用的房间或主数据不存在或已停用"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("排课计划不存在"),
                        "409": error_response("已发布的计划不可修改条目")
                    }
                }
            },
            "/admin/schedule-plans/{id}/publish": {
                "post": {
                    "tags": ["SchedulePlans"],
                    "summary": "发布计划：桥接写入 legacy 课表并重建排课实例（仅草稿；重复发布 409）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "发布结果（计划 + 重建统计）", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/PublishResultResponse" } } } },
                        "400": error_response("学期已停用"),
                        "403": error_response("缺少 schedule:manage"),
                        "404": error_response("排课计划不存在"),
                        "409": error_response("计划已发布（重复发布）")
                    }
                }
            },
            "/admin/ticket-scopes": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "工单数据范围：启用且拥有 ticket:read 的用户 + 区域范围",
                    "security": secured(),
                    "responses": {
                        "200": { "description": "用户范围列表", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/TicketScopeUser" } } } } },
                        "403": error_response("缺少 ticket:scope:manage / ticket:manage")
                    }
                }
            },
            "/admin/ticket-scopes/areas": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "工单范围配置用区域选项（不要求 area:read）",
                    "security": secured(),
                    "responses": {
                        "200": { "description": "区域选项", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/ScopeAreaOption" } } } } },
                        "403": error_response("缺少 ticket:scope:manage / ticket:manage")
                    }
                }
            },
            "/admin/ticket-scopes/{user_id}": {
                "put": {
                    "tags": ["Tickets"],
                    "summary": "整体替换某用户的工单区域范围（空数组 = 恢复不限制）",
                    "description": "权限：ticket:scope:manage 或 ticket:manage；区域必须存在。审计：ticket_scope.update。",
                    "security": secured(),
                    "parameters": [{ "name": "user_id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateTicketScopesRequest" } } } },
                    "responses": {
                        "200": { "description": "更新后的用户范围", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketScopeUser" } } } },
                        "403": error_response("缺少 ticket:scope:manage / ticket:manage"),
                        "404": error_response("用户或区域不存在")
                    }
                }
            },
            "/admin/tickets/{id}/attachments/upload": {
                "post": {
                    "tags": ["Tickets"],
                    "summary": "代理上传附件（请求体为文件原始字节）",
                    "description": "权限：ticket:attachment 或 ticket:handle；同源上传，避免对象存储 CORS。请求体 ≤20MiB（受 MAX_BODY_BYTES 限制），服务端计算 sha256 后写入对象存储并登记。Query：filename、content_type。",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "filename", "in": "query", "required": true, "schema": { "type": "string" } },
                        { "name": "content_type", "in": "query", "schema": { "type": "string" } }
                    ],
                    "requestBody": {
                        "required": true,
                        "content": { "application/octet-stream": { "schema": { "type": "string", "format": "binary" } } }
                    },
                    "responses": {
                        "201": { "description": "已上传并登记", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketAttachment" } } } },
                        "400": error_response("文件名/大小/sha256/类型不合法"),
                        "403": error_response("缺少 ticket:attachment / ticket:handle，或不在数据范围内"),
                        "404": error_response("工单不存在"),
                        "503": error_response("未配置对象存储或对象存储不可用")
                    }
                }
            },
            "/admin/tickets/{id}/attachments/presign": {
                "post": {
                    "tags": ["Tickets"],
                    "summary": "附件上传预签名（PUT URL）",
                    "description": "权限：ticket:attachment 或 ticket:handle；未配置对象存储 → 503。对象 key 为 tickets/{ticket_id}/{uuid}/{filename}。",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AttachmentPresignRequest" } } } },
                    "responses": {
                        "200": { "description": "预签名上传地址", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AttachmentPresignReply" } } } },
                        "400": error_response("文件名/大小/sha256/content-type 不合法"),
                        "403": error_response("缺少 ticket:attachment / ticket:handle，或工单不在数据范围内"),
                        "404": error_response("工单不存在"),
                        "503": error_response("未配置对象存储")
                    }
                }
            },
            "/admin/tickets/{id}/attachments": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "附件列表（含下载预签名；未配置存储时 download_url=null）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "附件列表", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/TicketAttachment" } } } } },
                        "403": error_response("缺少 ticket:read"),
                        "404": error_response("工单不存在或不在数据范围内")
                    }
                },
                "post": {
                    "tags": ["Tickets"],
                    "summary": "登记已上传的附件（object_key 必须属于该工单）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/AttachmentRegisterRequest" } } } },
                    "responses": {
                        "201": { "description": "已登记", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/TicketAttachment" } } } },
                        "400": error_response("元数据或 object_key 不合法"),
                        "403": error_response("缺少 ticket:attachment / ticket:handle，或工单不在数据范围内"),
                        "404": error_response("工单不存在"),
                        "409": error_response("object_key 已登记")
                    }
                }
            },
            "/admin/tickets/{id}/attachments/{attachment_id}/download": {
                "get": {
                    "tags": ["Tickets"],
                    "summary": "代理下载附件（Content-Disposition: attachment）",
                    "description": "权限：ticket:read + 工单数据范围；未配置对象存储时 503。",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "attachment_id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }
                    ],
                    "responses": {
                        "200": { "description": "附件二进制", "content": { "application/octet-stream": { "schema": { "type": "string", "format": "binary" } } } },
                        "403": error_response("缺少 ticket:read 或不在数据范围内"),
                        "404": error_response("工单或附件不存在"),
                        "503": error_response("未配置对象存储或对象存储不可用")
                    }
                }
            },
            "/admin/tickets/{id}/attachments/{attachment_id}": {
                "delete": {
                    "tags": ["Tickets"],
                    "summary": "删除附件（上传者或 ticket:manage）",
                    "security": secured(),
                    "parameters": [
                        { "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } },
                        { "name": "attachment_id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }
                    ],
                    "responses": {
                        "204": { "description": "已删除" },
                        "403": error_response("缺少权限、非上传者或不在数据范围内"),
                        "404": error_response("工单或附件不存在")
                    }
                }
            },
            "/admin/inspection-plans": {
                "get": {
                    "tags": ["TicketInspections"],
                    "summary": "巡检计划列表（含最近一次执行摘要）",
                    "security": secured(),
                    "responses": {
                        "200": { "description": "计划列表", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/InspectionPlanView" } } } } },
                        "403": error_response("缺少 ticket:inspection:manage / ticket:manage")
                    }
                },
                "post": {
                    "tags": ["TicketInspections"],
                    "summary": "创建巡检计划（global/area/room 范围 + 1..=168 小时间隔）",
                    "security": secured(),
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/CreateInspectionPlanRequest" } } } },
                    "responses": {
                        "201": { "description": "已创建", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/InspectionPlanView" } } } },
                        "400": error_response("名称/周期/范围不合法"),
                        "403": error_response("缺少 ticket:inspection:manage / ticket:manage"),
                        "404": error_response("区域或房间不存在")
                    }
                }
            },
            "/admin/inspection-plans/{id}": {
                "patch": {
                    "tags": ["TicketInspections"],
                    "summary": "修改巡检计划（部分更新）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "requestBody": { "required": true, "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UpdateInspectionPlanRequest" } } } },
                    "responses": {
                        "200": { "description": "更新后的计划", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/InspectionPlanView" } } } },
                        "400": error_response("名称/周期/范围不合法"),
                        "403": error_response("缺少 ticket:inspection:manage / ticket:manage"),
                        "404": error_response("巡检计划不存在")
                    }
                },
                "delete": {
                    "tags": ["TicketInspections"],
                    "summary": "删除巡检计划（执行历史级联删除）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "204": { "description": "已删除" },
                        "403": error_response("缺少 ticket:inspection:manage / ticket:manage"),
                        "404": error_response("巡检计划不存在")
                    }
                }
            },
            "/admin/inspection-plans/{id}/run": {
                "post": {
                    "tags": ["TicketInspections"],
                    "summary": "手动立即执行一次巡检（同步返回 run；不影响 next_run_at 周期）",
                    "security": secured(),
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                    "responses": {
                        "200": { "description": "执行结果（status=ok/issues/failed；有异常时 ticket_id 非空）", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/InspectionRun" } } } },
                        "403": error_response("缺少 ticket:inspection:manage / ticket:manage"),
                        "404": error_response("巡检计划不存在")
                    }
                }
            },
            "/admin/inspection-runs": {
                "get": {
                    "tags": ["TicketInspections"],
                    "summary": "执行历史（plan_id 可选；limit 1..=100，默认 20）",
                    "security": secured(),
                    "parameters": [
                        { "name": "plan_id", "in": "query", "schema": { "type": "string", "format": "uuid" } },
                        { "name": "limit", "in": "query", "schema": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 } }
                    ],
                    "responses": {
                        "200": { "description": "执行记录", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/InspectionRun" } } } } },
                        "400": error_response("limit 不合法"),
                        "403": error_response("缺少 ticket:inspection:manage / ticket:manage")
                    }
                }
            },
        }
    })
}

/// HTTP 端点：`GET /api-docs/openapi.json`。
pub async fn json_spec() -> Json<Value> {
    Json(spec())
}

pub async fn ui() -> Html<&'static str> {
    Html(DOCS_HTML)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spec_lists_core_paths_and_security() {
        let Json(spec) = json_spec().await;
        let paths = spec["paths"].as_object().expect("paths");
        assert!(paths.contains_key("/auth/login"));
        assert!(paths.contains_key("/auth/sessions"));
        assert!(paths.contains_key("/admin/audit-logs"));
        assert!(paths.contains_key("/announcements"));
        assert!(paths.contains_key("/admin/areas"));
        assert!(paths.contains_key("/admin/rooms/{id}/event-rules"));
        assert!(paths.contains_key("/admin/schedules/{id}/weekly-slots"));
        assert!(paths.contains_key("/admin/schedules/{id}/occurrences"));
        assert!(paths.contains_key("/admin/schedule-board"));
        assert!(paths.contains_key("/admin/schedules/ensure"));
        assert!(paths.contains_key("/admin/schedule-events/{id}/deliveries/{delivery_id}/retry"));
        assert!(paths.contains_key("/admin/devices"));
        assert!(paths.contains_key("/admin/devices/{serial}"));
        assert!(paths.contains_key("/admin/rooms/{id}/device"));
        assert!(paths.contains_key("/admin/rooms/{id}/device-token"));
        assert!(paths.contains_key("/admin/rooms/{id}/devices"));
        assert!(paths.contains_key("/admin/rooms/{id}/devices/token"));
        assert!(paths.contains_key("/admin/rooms/{id}/devices/{slot}"));
        assert!(paths.contains_key("/admin/storage/objects/presign"));
        assert!(paths.contains_key("/admin/storage/objects"));
        assert!(paths.contains_key("/admin/storage/objects/{sha256}"));
        assert!(paths.contains_key("/admin/images"));
        assert!(paths.contains_key("/admin/images/{id}/manifest"));
        assert!(paths.contains_key("/admin/images/{id}/verify"));
        assert!(paths.contains_key("/admin/images/{id}/publish"));
        assert!(paths.contains_key("/admin/rooms/{id}/images/{image_id}"));
        assert!(paths.contains_key("/admin/transfers"));
        assert!(paths.contains_key("/admin/transfers/{id}/retry"));
        assert!(paths.contains_key("/admin/collections"));
        assert!(paths.contains_key("/admin/agent-configs"));
        assert!(paths.contains_key("/admin/agent-configs/effective"));
        assert!(paths.contains_key("/admin/machines"));
        assert!(paths.contains_key("/admin/machines/{serial}"));
        assert!(paths.contains_key("/admin/machines/stream"));
        assert!(paths.contains_key("/admin/machines/{serial}/snapshots"));
        assert!(paths.contains_key("/machine/report"));
        assert!(paths.contains_key("/admin/device-events"));
        assert!(paths.contains_key("/admin/device-events/stream"));
        assert!(paths.contains_key("/admin/device-presence-deliveries"));
        // ---- 工单（P0）----
        assert!(paths.contains_key("/admin/tickets"));
        assert!(paths.contains_key("/admin/tickets/summary"));
        assert!(paths.contains_key("/admin/tickets/stats"));
        assert!(paths.contains_key("/admin/tickets/assignees"));
        assert!(paths.contains_key("/admin/tickets/{id}"));
        assert!(paths.contains_key("/admin/tickets/{id}/claim"));
        assert!(paths.contains_key("/admin/tickets/{id}/assign"));
        assert!(paths.contains_key("/admin/tickets/{id}/transition"));
        assert!(paths.contains_key("/admin/tickets/{id}/comments"));
        // ---- Wave 2：数据范围 / 附件 ----
        assert!(paths.contains_key("/admin/ticket-scopes"));
        assert!(paths.contains_key("/admin/ticket-scopes/areas"));
        assert!(paths.contains_key("/admin/ticket-scopes/{user_id}"));
        assert!(paths.contains_key("/admin/tickets/{id}/attachments"));
        assert!(paths.contains_key("/admin/tickets/{id}/attachments/presign"));
        assert!(paths.contains_key("/admin/tickets/{id}/attachments/upload"));
        assert!(paths.contains_key("/admin/tickets/{id}/attachments/{attachment_id}"));
        assert!(paths.contains_key("/admin/tickets/{id}/attachments/{attachment_id}/download"));
        // ---- Wave 3：巡检计划 ----
        assert!(paths.contains_key("/admin/inspection-plans"));
        assert!(paths.contains_key("/admin/inspection-plans/{id}"));
        assert!(paths.contains_key("/admin/inspection-plans/{id}/run"));
        assert!(paths.contains_key("/admin/inspection-runs"));
        assert!(paths.contains_key("/admin/tickets/export"));
        assert!(paths.contains_key("/admin/tickets/stream"));
        assert!(paths.contains_key("/admin/ticket-rules"));
        assert!(paths.contains_key("/admin/ticket-rules/{id}"));
        // ---- 排课全局模型（路径 2 Phase A）----
        for path in [
            "/admin/terms",
            "/admin/terms/{id}",
            "/admin/teachers",
            "/admin/teachers/{id}",
            "/admin/classes",
            "/admin/classes/{id}",
            "/admin/courses",
            "/admin/courses/{id}",
            "/admin/schedule-plans",
            "/admin/schedule-plans/{id}",
            "/admin/schedule-plans/{id}/clone",
            "/admin/schedule-plans/{id}/entries",
            "/admin/schedule-plans/{id}/publish",
        ] {
            assert!(paths.contains_key(path), "缺少路径：{path}");
        }
        let schemas = spec["components"]["schemas"].as_object().expect("schemas");
        for name in [
            "Ticket",
            "TicketPage",
            "TicketDetail",
            "TicketEvent",
            "TicketContext",
            "TicketSummary",
            "TicketStats",
            "TicketAssignee",
            "CreateTicketRequest",
            "UpdateTicketRequest",
            "AssignTicketRequest",
            "TransitionTicketRequest",
            "CommentTicketRequest",
            "TicketRule",
            "TicketRuleList",
            "CreateTicketRuleRequest",
            "UpdateTicketRuleRequest",
            "TicketStreamEvent",
            "TicketScopeUser",
            "UpdateTicketScopesRequest",
            "TicketAttachment",
            "AttachmentPresignRequest",
            "AttachmentPresignReply",
            "AttachmentRegisterRequest",
            "InspectionScopeType",
            "InspectionRunStatus",
            "InspectionPlan",
            "InspectionPlanView",
            "InspectionLastRun",
            "CreateInspectionPlanRequest",
            "UpdateInspectionPlanRequest",
            "InspectionRun",
        ] {
            assert!(schemas.contains_key(name), "缺少 schema：{name}");
        }
        // Phase A：主数据视图 / 计划 / 条目 / 发布结果与请求体。
        for name in [
            "TermView",
            "TeacherView",
            "ClassView",
            "CourseView",
            "PlanStatus",
            "SchedulePlan",
            "PlanEntry",
            "PlanEntryInput",
            "PlanDetailResponse",
            "PublishResultResponse",
            "TermPage",
            "TeacherPage",
            "ClassPage",
            "CoursePage",
            "SchedulePlanPage",
            "CreateTermRequest",
            "UpdateTermRequest",
            "CreateTeacherRequest",
            "UpdateTeacherRequest",
            "CreateClassRequest",
            "UpdateClassRequest",
            "CreateCourseRequest",
            "UpdateCourseRequest",
            "CreateSchedulePlanRequest",
            "UpdateSchedulePlanRequest",
            "ReplacePlanEntriesRequest",
        ] {
            assert!(schemas.contains_key(name), "缺少 schema：{name}");
        }
        // Wave 2：根因字段进入 Ticket/Transition 契约。
        assert!(
            schemas["Ticket"]["required"]
                .as_array()
                .expect("Ticket.required")
                .iter()
                .any(|value| value == "root_cause")
        );
        assert!(
            schemas["Ticket"]["properties"]["root_cause"]["oneOf"]
                .as_array()
                .expect("root_cause")
                .iter()
                .any(|value| value["enum"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|item| item == "hardware")))
        );
        assert!(
            schemas["TransitionTicketRequest"]["properties"]
                .get("root_cause")
                .is_some()
        );
        // PATCH 内容修改新增的时间线事件类型。
        assert!(
            schemas["TicketEventKind"]["enum"]
                .as_array()
                .expect("TicketEventKind enum")
                .iter()
                .any(|value| value == "updated")
        );
        // 详情契约：ticket + events + context。
        let detail = &schemas["TicketDetail"]["required"];
        assert!(
            detail
                .as_array()
                .expect("required")
                .iter()
                .any(|v| v == "context")
        );
        assert!(
            spec["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag["name"] == "Schedules")
        );
        assert!(
            spec["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag["name"] == "ScheduleBoard")
        );
        assert!(
            spec["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag["name"] == "Devices")
        );
        assert!(
            spec["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag["name"] == "Storage")
        );
        assert!(
            spec["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag["name"] == "Images")
        );
        assert!(
            spec["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag["name"] == "AgentConfigs")
        );
        assert!(
            spec["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag["name"] == "DeviceEvents")
        );
        assert!(
            spec["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag["name"] == "Tickets")
        );
        assert!(
            spec["tags"]
                .as_array()
                .expect("tags")
                .iter()
                .any(|tag| tag["name"] == "TicketInspections")
        );
        for tag in ["Terms", "Teachers", "Classes", "Courses", "SchedulePlans"] {
            assert!(
                spec["tags"]
                    .as_array()
                    .expect("tags")
                    .iter()
                    .any(|t| t["name"] == tag),
                "缺少 tag：{tag}"
            );
        }
        assert_eq!(
            spec["components"]["securitySchemes"]["bearerAuth"]["scheme"],
            "bearer"
        );
    }

    #[tokio::test]
    async fn docs_ui_points_to_the_spec() {
        let Html(html) = ui().await;
        assert!(html.contains("/api-docs/openapi.json"));
    }
}
