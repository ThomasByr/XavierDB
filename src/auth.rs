//! Authentication: Argon2id credential hashing, JWT sign/verify, admin
//! dashboard sessions and the /auth brute-force throttle.
//!
//! Why JWT: verifying a JWT is a single HMAC-SHA256 — microseconds, no disk,
//! no shared memory. Every worker can verify any token because the secret is
//! in process memory. Argon2id (the slow, strong hash) is only paid once per
//! login on /auth, never on the hot /q path.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Argon2, Params};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiErrorKind};
use crate::state::AppState;

pub const ARGON2_M: u32 = 65536; // 64 MiB
pub const ARGON2_T: u32 = 3;
pub const ARGON2_P: u32 = 4;

/// Fixed dummy PHC string (Argon2id, same params as real credentials).
/// /auth and the dashboard login verify against it when the app/username is
/// unknown, so response timing doesn't reveal whether an identity exists.
pub const DUMMY_PHC: &str = "$argon2id$v=19$m=65536,t=3,p=4$eOh17UFkMYH4DA6BpuZKtw$Wc0TNh3975QklccVwNHAh7Dpt44n7k2tMyajN9y02Dw";

// ---------------------------------------------------------------------------
// Argon2id
// ---------------------------------------------------------------------------

pub fn argon_params() -> Params {
    Params::new(ARGON2_M, ARGON2_T, ARGON2_P, Some(32)).expect("valid argon2 params")
}

/// Hash a plaintext credential into a PHC string ("$argon2id$...").
pub fn hash_credential(secret: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    Argon2::new(
        argon2::Algorithm::Argon2id,
        argon2::Version::V0x13,
        argon_params(),
    )
    .hash_password(secret.as_bytes(), &salt)
    .map(|h| h.to_string())
    .map_err(|e| format!("hashing failed: {e}"))
}

/// Verify a plaintext credential against a stored PHC string.
pub fn verify_credential(secret: &str, phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    Argon2::default()
        .verify_password(secret.as_bytes(), &parsed)
        .is_ok()
}

// ---------------------------------------------------------------------------
// JWT
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// name_id
    pub sub: String,
    /// app_id
    pub app: String,
    pub iat: i64,
    pub exp: i64,
    /// unique token id
    pub jti: String,
}

pub fn sign_jwt(
    state: &AppState,
    name: &str,
    app: &str,
    lifetime_minutes: u64,
) -> Result<String, ApiError> {
    let now = crate::state::now_ms() / 1000;
    let claims = Claims {
        sub: name.to_string(),
        app: app.to_string(),
        iat: now,
        exp: now + (lifetime_minutes as i64) * 60,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(&state.jwt_secret),
    )
    .map_err(|e| ApiError::internal(format!("jwt sign failed: {e}")))
}

pub fn verify_jwt(state: &AppState, token: &str) -> Result<Claims, ApiError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.leeway = 5;
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(&state.jwt_secret),
        &validation,
    )
    .map(|d| d.claims)
    .map_err(|_| ApiError::unauthorized())
}

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// Split "name@app" into (name, app). Returns None when malformed.
pub fn parse_identifier(id: &str) -> Option<(String, String)> {
    let (name, app) = id.rsplit_once('@')?;
    if name.is_empty() || app.is_empty() {
        return None;
    }
    if name.len() > 64 || app.len() > 64 {
        return None;
    }
    let ok = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:~".contains(c))
    };
    if !ok(name) || !ok(app) {
        return None;
    }
    Some((name.to_string(), app.to_string()))
}

/// Validate database / collection path segments.
pub fn valid_path_segment(seg: &str, is_db: bool) -> bool {
    if seg.is_empty() || seg.len() > 120 {
        return false;
    }
    seg.chars().all(|c| {
        c != '\0'
            && c != '/'
            && c != '\\'
            && c != '$'
            && c != '"'
            && c != '\''
            && !(is_db && c == '.')
    })
}

// ---------------------------------------------------------------------------
// Admin dashboard sessions
// ---------------------------------------------------------------------------

pub fn create_admin_session(state: &AppState, user: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let ttl_hours = state
        .config
        .read()
        .map(|c| c.auth.session_ttl_hours)
        .unwrap_or(24);
    let expires = crate::state::now_ms() + (ttl_hours as i64) * 3600 * 1000;
    state.sessions.insert(
        token.clone(),
        crate::state::AdminSession {
            user: user.to_string(),
            expires_ms: expires,
        },
    );
    token
}

pub fn check_admin_session(state: &AppState, token: &str) -> Result<String, ApiError> {
    let Some(sess) = state.sessions.get(token) else {
        return Err(ApiError::unauthorized());
    };
    if sess.expires_ms < crate::state::now_ms() {
        drop(sess);
        state.sessions.remove(token);
        return Err(ApiError::unauthorized());
    }
    Ok(sess.user.clone())
}

// ---------------------------------------------------------------------------
// /auth brute-force throttle (fixed 1-minute window per IP)
// ---------------------------------------------------------------------------

pub fn auth_throttled(state: &AppState, ip: &str) -> Result<(), ApiError> {
    let max = state
        .config
        .read()
        .map(|c| c.auth.max_per_minute_per_ip)
        .unwrap_or(30);
    let now = crate::state::now_ms();
    let window = now / 60_000;
    let mut entry = state
        .auth_throttle
        .entry(ip.to_string())
        .or_insert((window, 0));
    if entry.0 != window {
        *entry = (window, 1);
        return Ok(());
    }
    entry.1 += 1;
    if entry.1 > max {
        return Err(ApiError::new(
            ApiErrorKind::TooManyRequests,
            "too many authentication attempts, slow down",
        ));
    }
    Ok(())
}

pub fn throttle_sweep(state: &AppState) {
    let window = crate::state::now_ms() / 60_000;
    state
        .auth_throttle
        .retain(|_, (w, _)| *w == window || *w + 2 >= window);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_parsing() {
        assert_eq!(
            parse_identifier("user1@provider1"),
            Some(("user1".to_string(), "provider1".to_string()))
        );
        // '@' is not allowed inside a name/app, so this is invalid
        assert!(parse_identifier("user1@provider1@x").is_none());
        assert!(parse_identifier("noapp").is_none());
        assert!(parse_identifier("@app").is_none());
        assert!(parse_identifier("a@b c").is_none());
    }

    #[test]
    fn cred_roundtrip() {
        let phc = hash_credential("hunter2").unwrap();
        assert!(phc.starts_with("$argon2id$"));
        assert!(verify_credential("hunter2", &phc));
        assert!(!verify_credential("hunter3", &phc));
    }

    #[test]
    fn dummy_phc_parses_and_rejects() {
        // the dummy hash must be a VALID PHC string: if it failed to parse,
        // unknown-identity logins would return fast again (timing oracle)
        assert!(!verify_credential("any-wrong-password", DUMMY_PHC));
    }

    #[test]
    fn path_segments() {
        assert!(valid_path_segment("db1", true));
        assert!(!valid_path_segment("db.1", true));
        assert!(valid_path_segment("coll.1", false));
        assert!(!valid_path_segment("a/b", false));
        assert!(!valid_path_segment("a$b", false));
        assert!(!valid_path_segment("", false));
    }
}
