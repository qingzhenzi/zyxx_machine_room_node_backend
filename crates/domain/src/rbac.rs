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

    pub const ALL: &[&str] = &[
        ANNOUNCEMENT_READ,
        ANNOUNCEMENT_WRITE,
        ANNOUNCEMENT_PUBLISH,
        ANNOUNCEMENT_DELETE,
        USER_MANAGE,
        ROLE_MANAGE,
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
}
