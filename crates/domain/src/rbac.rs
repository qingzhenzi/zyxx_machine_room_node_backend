//! RBAC 的权限点、系统角色与判定逻辑。
//!
//! 匹配算法在 `qe-auth`（`qe_auth::permission`）；这里保留业务侧的权限点常量
//! 与角色编码，并原样转出判定函数，避免调用方改路径。

pub use qe_auth::permission::{has_any_permission, has_permission};

/// 权限编码约定：`<资源>:<动作>`。
pub mod permissions {
    pub const ANNOUNCEMENT_READ: &str = "announcement:read";
    pub const ANNOUNCEMENT_WRITE: &str = "announcement:write";
    pub const ANNOUNCEMENT_PUBLISH: &str = "announcement:publish";
    pub const ANNOUNCEMENT_DELETE: &str = "announcement:delete";
    pub const USER_MANAGE: &str = "user:manage";
    pub const ROLE_MANAGE: &str = "role:manage";
    pub const AUDIT_READ: &str = "audit:read";
    pub const AREA_READ: &str = "area:read";
    pub const AREA_MANAGE: &str = "area:manage";
    pub const ROOM_READ: &str = "room:read";
    pub const ROOM_MANAGE: &str = "room:manage";
    pub const SCHEDULE_READ: &str = "schedule:read";
    pub const SCHEDULE_MANAGE: &str = "schedule:manage";
    pub const SCHEDULE_EVENT_READ: &str = "schedule:event:read";
    pub const SCHEDULE_EVENT_MANAGE: &str = "schedule:event:manage";
    pub const DEVICE_READ: &str = "device:read";
    pub const DEVICE_MANAGE: &str = "device:manage";
    pub const STORAGE_READ: &str = "storage:read";
    pub const STORAGE_MANAGE: &str = "storage:manage";
    pub const IMAGE_READ: &str = "image:read";
    pub const IMAGE_MANAGE: &str = "image:manage";
    pub const TICKET_READ: &str = "ticket:read";
    pub const TICKET_CREATE: &str = "ticket:create";
    pub const TICKET_HANDLE: &str = "ticket:handle";
    pub const TICKET_MANAGE: &str = "ticket:manage";
    // P1 Wave 1：独立细粒度权限（与 handle/manage 做 OR 兼容）。
    pub const TICKET_CLAIM: &str = "ticket:claim";
    pub const TICKET_COMMENT: &str = "ticket:comment";
    pub const TICKET_TRANSITION: &str = "ticket:transition";
    pub const TICKET_ASSIGN: &str = "ticket:assign";
    pub const TICKET_UPDATE: &str = "ticket:update";
    pub const TICKET_RULE_MANAGE: &str = "ticket:rule:manage";
    pub const TICKET_EXPORT: &str = "ticket:export";
    // Wave 2：区域数据范围 + 附件。
    pub const TICKET_SCOPE_MANAGE: &str = "ticket:scope:manage";
    pub const TICKET_SCOPE_ALL: &str = "ticket:scope:all";
    pub const TICKET_ATTACHMENT: &str = "ticket:attachment";
    pub const TICKET_INSPECTION_MANAGE: &str = "ticket:inspection:manage";

    pub const ALL: &[&str] = &[
        ANNOUNCEMENT_READ,
        ANNOUNCEMENT_WRITE,
        ANNOUNCEMENT_PUBLISH,
        ANNOUNCEMENT_DELETE,
        USER_MANAGE,
        ROLE_MANAGE,
        AUDIT_READ,
        AREA_READ,
        AREA_MANAGE,
        ROOM_READ,
        ROOM_MANAGE,
        SCHEDULE_READ,
        SCHEDULE_MANAGE,
        SCHEDULE_EVENT_READ,
        SCHEDULE_EVENT_MANAGE,
        DEVICE_READ,
        DEVICE_MANAGE,
        STORAGE_READ,
        STORAGE_MANAGE,
        IMAGE_READ,
        IMAGE_MANAGE,
        TICKET_READ,
        TICKET_CREATE,
        TICKET_HANDLE,
        TICKET_MANAGE,
        TICKET_CLAIM,
        TICKET_COMMENT,
        TICKET_TRANSITION,
        TICKET_ASSIGN,
        TICKET_UPDATE,
        TICKET_RULE_MANAGE,
        TICKET_EXPORT,
        TICKET_SCOPE_MANAGE,
        TICKET_SCOPE_ALL,
        TICKET_ATTACHMENT,
        TICKET_INSPECTION_MANAGE,
    ];
}

/// 系统内置角色编码。`admin` 是受保护的系统角色。
pub mod roles {
    pub const ADMIN: &str = "admin";
    pub const EDITOR: &str = "editor";
    pub const VIEWER: &str = "viewer";
}

pub fn is_admin(role_codes: &[String]) -> bool {
    role_codes.iter().any(|code| code.as_str() == roles::ADMIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_role_is_recognized() {
        let roles = vec![roles::EDITOR.to_owned(), roles::ADMIN.to_owned()];
        assert!(is_admin(&roles));
        assert!(!is_admin(&[roles::VIEWER.to_owned()]));
    }

    #[test]
    fn schedule_permissions_are_registered() {
        for code in [
            permissions::AREA_READ,
            permissions::AREA_MANAGE,
            permissions::ROOM_READ,
            permissions::ROOM_MANAGE,
            permissions::SCHEDULE_READ,
            permissions::SCHEDULE_MANAGE,
            permissions::SCHEDULE_EVENT_READ,
            permissions::SCHEDULE_EVENT_MANAGE,
            permissions::DEVICE_READ,
            permissions::DEVICE_MANAGE,
        ] {
            assert!(permissions::ALL.contains(&code), "缺少权限：{code}");
        }
        assert_eq!(permissions::ALL.len(), 36);
    }

    #[test]
    fn ticket_permissions_are_registered() {
        for code in [
            permissions::TICKET_READ,
            permissions::TICKET_CREATE,
            permissions::TICKET_HANDLE,
            permissions::TICKET_MANAGE,
        ] {
            assert!(permissions::ALL.contains(&code), "缺少权限：{code}");
        }
        assert_eq!(permissions::TICKET_READ, "ticket:read");
        assert_eq!(permissions::TICKET_CREATE, "ticket:create");
        assert_eq!(permissions::TICKET_HANDLE, "ticket:handle");
        assert_eq!(permissions::TICKET_MANAGE, "ticket:manage");
        assert_eq!(permissions::ALL.len(), 36);
    }

    #[test]
    fn p1_ticket_permissions_are_registered() {
        for (code, expected) in [
            (permissions::TICKET_CLAIM, "ticket:claim"),
            (permissions::TICKET_COMMENT, "ticket:comment"),
            (permissions::TICKET_TRANSITION, "ticket:transition"),
            (permissions::TICKET_ASSIGN, "ticket:assign"),
            (permissions::TICKET_UPDATE, "ticket:update"),
            (permissions::TICKET_RULE_MANAGE, "ticket:rule:manage"),
            (permissions::TICKET_EXPORT, "ticket:export"),
        ] {
            assert!(permissions::ALL.contains(&code), "缺少权限：{code}");
            assert_eq!(code, expected);
        }
        assert_eq!(permissions::ALL.len(), 36);
    }

    #[test]
    fn wave2_ticket_permissions_are_registered() {
        for (code, expected) in [
            (permissions::TICKET_SCOPE_MANAGE, "ticket:scope:manage"),
            (permissions::TICKET_SCOPE_ALL, "ticket:scope:all"),
            (permissions::TICKET_ATTACHMENT, "ticket:attachment"),
            (
                permissions::TICKET_INSPECTION_MANAGE,
                "ticket:inspection:manage",
            ),
        ] {
            assert!(permissions::ALL.contains(&code), "缺少权限：{code}");
            assert_eq!(code, expected);
        }
        assert_eq!(permissions::ALL.len(), 36);
    }

    #[test]
    fn storage_and_image_permissions_are_registered() {
        for code in [
            permissions::STORAGE_READ,
            permissions::STORAGE_MANAGE,
            permissions::IMAGE_READ,
            permissions::IMAGE_MANAGE,
        ] {
            assert!(permissions::ALL.contains(&code), "缺少权限：{code}");
        }
        assert_eq!(permissions::STORAGE_READ, "storage:read");
        assert_eq!(permissions::STORAGE_MANAGE, "storage:manage");
        assert_eq!(permissions::IMAGE_READ, "image:read");
        assert_eq!(permissions::IMAGE_MANAGE, "image:manage");
    }
}
