//! `qe-auth` 的 PostgreSQL 端口实现：权限事实来源。

use qe_auth::{AuthError, AuthResult};
use uuid::Uuid;

use crate::{error::DbError, users::UserRepo};

fn as_auth_error(error: DbError) -> AuthError {
    AuthError::internal(error.to_string())
}

impl qe_auth::permission::PermissionSource for UserRepo {
    async fn permissions_of(&self, user_id: Uuid) -> AuthResult<Vec<String>> {
        UserRepo::permissions_of(self, user_id)
            .await
            .map_err(as_auth_error)
    }
}
