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
