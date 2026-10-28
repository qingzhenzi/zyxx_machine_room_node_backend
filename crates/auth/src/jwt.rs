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

    pub fn issue_access_token(&self, user_id: Uuid, ttl_secs: i64) -> AuthResult<String> {
        let now = Utc::now().timestamp();
        let claims = Claims {
            sub: user_id.to_string(),
            iat: now,
            exp: now + ttl_secs.max(1),
            jti: Uuid::new_v4().to_string(),
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding)
            .map_err(|error| AuthError::internal(format!("jwt encode error: {error}")))
    }

    pub fn verify_access_token(&self, token: &str) -> AuthResult<Uuid> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;
        validation.leeway = 5;

        let token_data = decode::<Claims>(token, &self.decoding, &validation)
            .map_err(|_| AuthError::unauthorized("访问令牌无效或已过期"))?;
        Uuid::parse_str(&token_data.claims.sub).map_err(|_| AuthError::unauthorized("访问令牌无效"))
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
        let token = keys.issue_access_token(user_id, 60).unwrap();
        assert_eq!(keys.verify_access_token(&token).unwrap(), user_id);
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
        let token = issuer.issue_access_token(Uuid::new_v4(), 60).unwrap();
        assert!(verifier.verify_access_token(&token).is_err());
    }
}
