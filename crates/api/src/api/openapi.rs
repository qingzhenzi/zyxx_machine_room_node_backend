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

pub async fn json_spec() -> Json<Value> {
    Json(json!({
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
            { "name": "ScheduleEvents", "description": "触发规则、事件与投递重放/重试" },
            { "name": "Devices", "description": "设备：注册状态、在线状态与区域/房间绑定（v3）" },
            { "name": "DeviceEvents", "description": "设备在线事件：历史查询、SSE 实时流与可靠投递记录（P3）" },
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
            }
        }
    }))
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
