use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AuthError, AuthResult};

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    iat: i64,
    exp: i64,
    jti: String,
    /// 设备会话 ID。旧版本令牌没有该字段，迁移期内仍可验证。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sid: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedToken {
    pub user_id: Uuid,
    /// None 表示旧版 access token；调用方可选择兼容或拒绝。
    pub session_id: Option<Uuid>,
}

/// 签发与校验访问令牌。密钥只在进程内保存。
pub struct JwtKeys {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl JwtKeys {
    pub fn new(secret: &str) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret.as_bytes()),
            decoding: DecodingKey::from_secret(secret.as_bytes()),
        }
    }

    pub fn issue_access_token(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        ttl_secs: i64,
    ) -> AuthResult<String> {
        let now = Utc::now().timestamp();
        let claims = Claims {
            sub: user_id.to_string(),
            iat: now,
            exp: now + ttl_secs.max(1),
            jti: Uuid::new_v4().to_string(),
            sid: Some(session_id.to_string()),
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(|error| AuthError::internal(format!("jwt encode error: {error}")))
    }

    pub fn verify_access_token(&self, token: &str) -> AuthResult<VerifiedToken> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        validation.leeway = 5;

        let token_data = decode::<Claims>(token, &self.decoding, &validation)
            .map_err(|_| AuthError::unauthorized("访问令牌无效或已过期"))?;
        let user_id = Uuid::parse_str(&token_data.claims.sub)
            .map_err(|_| AuthError::unauthorized("访问令牌无效"))?;
        let session_id = token_data
            .claims
            .sid
            .as_deref()
            .map(Uuid::parse_str)
            .transpose()
            .map_err(|_| AuthError::unauthorized("访问令牌会话无效"))?;

        Ok(VerifiedToken {
            user_id,
            session_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{Claims, JwtKeys};
    use crate::error::AuthError;
    use chrono::Utc;
    use jsonwebtoken::{Algorithm, Header, encode};
    use uuid::Uuid;

    const SECRET: &str = "test-secret-test-secret-test-secret-1234";

    #[test]
    fn issued_token_can_be_verified() {
        let keys = JwtKeys::new(SECRET);
        let user_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let token = keys.issue_access_token(user_id, session_id, 60).unwrap();
        let verified = keys.verify_access_token(&token).unwrap();
        assert_eq!(verified.user_id, user_id);
        assert_eq!(verified.session_id, Some(session_id));
    }

    #[test]
    fn legacy_token_without_session_still_verifies() {
        let keys = JwtKeys::new(SECRET);
        let now = Utc::now().timestamp();
        let claims = Claims {
            sub: Uuid::new_v4().to_string(),
            iat: now,
            exp: now + 60,
            jti: "test".to_owned(),
            sid: None,
        };
        let token = encode(&Header::new(Algorithm::HS256), &claims, &keys.encoding).unwrap();
        assert_eq!(keys.verify_access_token(&token).unwrap().session_id, None);
    }

    #[test]
    fn expired_token_is_rejected() {
        let keys = JwtKeys::new(SECRET);
        let now = Utc::now().timestamp();
        let claims = Claims {
            sub: Uuid::new_v4().to_string(),
            iat: now - 200,
            exp: now - 100,
            jti: "test".to_owned(),
            sid: None,
        };
        let token = encode(&Header::new(Algorithm::HS256), &claims, &keys.encoding).unwrap();

        assert!(matches!(
            keys.verify_access_token(&token),
            Err(AuthError::Unauthorized(_))
        ));
    }

    #[test]
    fn token_signed_with_another_secret_is_rejected() {
        let issuer = JwtKeys::new(SECRET);
        let verifier = JwtKeys::new("another-secret-another-secret-123456");
        let token = issuer
            .issue_access_token(Uuid::new_v4(), Uuid::new_v4(), 60)
            .unwrap();
        assert!(verifier.verify_access_token(&token).is_err());
    }
}
