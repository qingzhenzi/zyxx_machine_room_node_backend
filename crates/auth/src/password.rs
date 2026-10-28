use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};

use crate::error::{AuthError, AuthResult};

pub fn hash_password(password: &str) -> AuthResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string();
    Ok(hash)
}

pub fn verify_password(password: &str, password_hash: &str) -> bool {
    let Ok(parsed_hash) = PasswordHash::new(password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

/// Argon2 是 CPU 密集型操作，放到阻塞线程池，避免卡住异步运行时。
pub async fn hash_password_async(password: String) -> AuthResult<String> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(|error| AuthError::internal(format!("password hash task failed: {error}")))?
}

pub async fn verify_password_async(password: String, password_hash: String) -> AuthResult<bool> {
    tokio::task::spawn_blocking(move || verify_password(&password, &password_hash))
        .await
        .map_err(|error| AuthError::internal(format!("password verify task failed: {error}")))
}

#[cfg(test)]
mod tests {
    use super::{hash_password, verify_password};

    #[test]
    fn hash_is_salted_and_verifiable() {
        let first = hash_password("correct horse battery staple").unwrap();
        let second = hash_password("correct horse battery staple").unwrap();
        assert_ne!(first, second, "同明文应产生不同盐值");
        assert!(verify_password("correct horse battery staple", &first));
        assert!(!verify_password("wrong password", &first));
    }

    #[test]
    fn malformed_hash_is_rejected() {
        assert!(!verify_password("anything", "not-a-phc-string"));
    }
}
