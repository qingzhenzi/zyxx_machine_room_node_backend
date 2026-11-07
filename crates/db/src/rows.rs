//! SQLx 行模型与领域模型之间的映射。
//!
//! 领域模型不认识数据库；`FromRow` 只出现在这里。

use chrono::{DateTime, Utc};
use qe_domain::models::{
    Announcement, AnnouncementStatus, AuditLog, Permission, Role, RoleSummary, User, UserSession,
    UserSummary,
};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Clone, FromRow)]
pub struct UserRow {
    pub id: Uuid,
    pub username: String,
    pub email: Option<String>,
    pub display_name: String,
    pub password_hash: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        Self {
            id: row.id,
            username: row.username,
            email: row.email,
            display_name: row.display_name,
            password_hash: row.password_hash,
            is_active: row.is_active,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct UserSummaryRow {
    pub id: Uuid,
    pub username: String,
    pub email: Option<String>,
    pub display_name: String,
    pub is_active: bool,
    pub roles: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<UserSummaryRow> for UserSummary {
    fn from(row: UserSummaryRow) -> Self {
        Self {
            id: row.id,
            username: row.username,
            email: row.email,
            display_name: row.display_name,
            is_active: row.is_active,
            roles: row.roles,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct RoleRow {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: String,
    pub is_system: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<RoleRow> for Role {
    fn from(row: RoleRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            name: row.name,
            description: row.description,
            is_system: row.is_system,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct RoleSummaryRow {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: String,
    pub is_system: bool,
    pub permissions: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<RoleSummaryRow> for RoleSummary {
    fn from(row: RoleSummaryRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            name: row.name,
            description: row.description,
            is_system: row.is_system,
            permissions: row.permissions,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct PermissionRow {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

impl From<PermissionRow> for Permission {
    fn from(row: PermissionRow) -> Self {
        Self {
            id: row.id,
            code: row.code,
            name: row.name,
            description: row.description,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct AnnouncementRow {
    pub id: Uuid,
    pub title: String,
    pub content: String,
    pub status: String,
    pub is_pinned: bool,
    pub author_id: Option<Uuid>,
    pub author_name: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<AnnouncementRow> for Announcement {
    fn from(row: AnnouncementRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            content: row.content,
            status: row
                .status
                .parse::<AnnouncementStatus>()
                .unwrap_or(AnnouncementStatus::Draft),
            is_pinned: row.is_pinned,
            author_id: row.author_id,
            author_name: row.author_name,
            published_at: row.published_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct AuditLogRow {
    pub id: Uuid,
    pub actor_id: Option<Uuid>,
    pub actor_username: String,
    pub actor_display_name: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub detail: serde_json::Value,
    pub ip: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<AuditLogRow> for AuditLog {
    fn from(row: AuditLogRow) -> Self {
        Self {
            id: row.id,
            actor_id: row.actor_id,
            actor_username: row.actor_username,
            actor_display_name: row.actor_display_name,
            action: row.action,
            resource_type: row.resource_type,
            resource_id: row.resource_id,
            detail: row.detail,
            ip: row.ip,
            created_at: row.created_at,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct UserSessionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub user_agent: String,
    pub ip: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<UserSessionRow> for UserSession {
    fn from(row: UserSessionRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            user_agent: row.user_agent,
            ip: row.ip,
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
            revoked_at: row.revoked_at,
        }
    }
}

/// `federation_nodes` 行（总控阶段 2）。
#[derive(Debug, Clone, FromRow)]
pub struct FederationNodeRow {
    pub id: Uuid,
    pub name: String,
    pub base_url: String,
    pub token: String,
    pub enabled: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_status: Option<String>,
    pub cached_summary: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
