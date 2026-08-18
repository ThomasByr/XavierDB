//! API error types with meaningful HTTP status codes and sanitized messages.

use axum::Json;
use axum::extract::{FromRequest, FromRequestParts, Query};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::de::DeserializeOwned;
use serde_json::json;

#[derive(Debug, Clone)]
pub enum ApiErrorKind {
    /// 400 — malformed input (bad filter JSON, bad limit, ...)
    BadRequest,
    /// 400 — invalid cursor payload
    InvalidCursor,
    /// 400 — invalid filter syntax
    InvalidFilter,
    /// 400 — invalid sort syntax
    InvalidSort,
    /// 400 — invalid limit value
    InvalidLimit,
    /// 400 — invalid projection syntax
    InvalidProjection,
    /// 401 — missing / invalid / expired credentials
    Unauthorized,
    /// 403 — identifier is blocked
    Blocked,
    /// 403 — permission denied for this operation
    Forbidden,
    /// 404 — resource not found
    NotFound,
    /// 429 — too many attempts (auth throttling)
    TooManyRequests,
    /// 409 — duplicate key / unique constraint
    Conflict,
    /// 429 — adaptive limit enforcement (kept for future use)
    /// 500 — unexpected server error
    Internal,
    /// 503 — MongoDB unreachable / degraded
    Unavailable,
    /// 504 — a database operation exceeded the configured deadline
    /// (server.yml runtime.find_timeout_ms)
    Timeout,
}

impl ApiErrorKind {
    pub fn status(&self) -> StatusCode {
        use ApiErrorKind::*;
        match self {
            BadRequest | InvalidCursor | InvalidFilter | InvalidSort | InvalidLimit
            | InvalidProjection => StatusCode::BAD_REQUEST,
            Unauthorized => StatusCode::UNAUTHORIZED,
            Blocked | Forbidden => StatusCode::FORBIDDEN,
            NotFound => StatusCode::NOT_FOUND,
            Conflict => StatusCode::CONFLICT,
            TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Internal => StatusCode::INTERNAL_SERVER_ERROR,
            Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Timeout => StatusCode::GATEWAY_TIMEOUT,
        }
    }

    pub fn code(&self) -> &'static str {
        use ApiErrorKind::*;
        match self {
            BadRequest => "BAD_REQUEST",
            InvalidCursor => "INVALID_CURSOR",
            InvalidFilter => "INVALID_FILTER",
            InvalidSort => "INVALID_SORT",
            InvalidLimit => "INVALID_LIMIT",
            InvalidProjection => "INVALID_PROJECTION",
            Unauthorized => "UNAUTHORIZED",
            Blocked => "BLOCKED",
            Forbidden => "FORBIDDEN",
            NotFound => "NOT_FOUND",
            Conflict => "CONFLICT",
            TooManyRequests => "TOO_MANY_REQUESTS",
            Internal => "INTERNAL_ERROR",
            Unavailable => "UNAVAILABLE",
            Timeout => "TIMEOUT",
        }
    }
}

/// Removes sensitive tokens (filesystem paths, IPv4/IPv6 addresses) from a
/// message before it can ever reach a client. Bare hostnames and host:port
/// pairs (e.g. `localhost:27017`) are NOT scrubbed — they are deployment
/// configuration, not secrets.
pub fn sanitize(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let chars: Vec<char> = msg.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // path-ish token: '/', '\', or drive-letter prefix (C:\, C:/) —
        // consumes until whitespace/quote; covers unix paths, UNC and URLs
        let drive = chars[i].is_ascii_alphabetic()
            && i + 1 < chars.len()
            && chars[i + 1] == ':'
            && i + 2 < chars.len()
            && (chars[i + 2] == '\\' || chars[i + 2] == '/');
        if chars[i] == '/' || chars[i] == '\\' || drive {
            let mut j = if drive { i + 3 } else { i };
            while j < chars.len()
                && !chars[j].is_whitespace()
                && chars[j] != '"'
                && chars[j] != '\''
            {
                j += 1;
            }
            out.push_str("<path>");
            i = j;
            continue;
        }
        // bracketed IPv6: [::1]:27017
        if chars[i] == '[' {
            let mut j = i;
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j < chars.len() {
                out.push_str("<ip>");
                i = j + 1;
                continue;
            }
        }
        // bare IPv6-ish token: at least two ':' and mostly hex
        if chars[i].is_ascii_hexdigit() || chars[i] == ':' {
            let mut j = i;
            let mut colons = 0;
            while j < chars.len()
                && (chars[j].is_ascii_hexdigit() || chars[j] == ':' || chars[j] == '.')
            {
                if chars[j] == ':' {
                    colons += 1;
                }
                j += 1;
            }
            if colons >= 2 && j > i + 2 {
                out.push_str("<ip>");
                i = j;
                continue;
            }
        }
        // IPv4
        if chars[i].is_ascii_digit() {
            let mut j = i;
            let mut dots = 0;
            while j < chars.len() && (chars[j].is_ascii_digit() || chars[j] == '.') {
                if chars[j] == '.' {
                    dots += 1;
                }
                j += 1;
            }
            if dots == 3 && j > i + 3 {
                out.push_str("<ip>");
                i = j;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[derive(Debug)]
pub struct ApiError {
    pub kind: ApiErrorKind,
    pub message: String,
}

impl ApiError {
    pub fn new(kind: ApiErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self::new(ApiErrorKind::BadRequest, msg)
    }
    pub fn unauthorized() -> Self {
        Self::new(
            ApiErrorKind::Unauthorized,
            "missing or invalid authentication token",
        )
    }
    pub fn blocked() -> Self {
        Self::new(ApiErrorKind::Blocked, "this identifier is blocked")
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(ApiErrorKind::NotFound, msg)
    }
    pub fn internal(msg: impl Into<String>) -> Self {
        Self::new(ApiErrorKind::Internal, msg)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.kind.status();
        let body = Json(json!({
            "error": sanitize(&self.message),
            "code": self.kind.code(),
            "status": status.as_u16(),
        }));
        (status, body).into_response()
    }
}

impl From<mongodb::error::Error> for ApiError {
    fn from(e: mongodb::error::Error) -> Self {
        use mongodb::error::ErrorKind;
        // client-caused command failures: bad regex/shapes/validation and
        // duplicate keys must not surface as 500 INTERNAL_ERROR
        let is_client_command =
            |code: i32| matches!(code, 2 | 9 | 14 | 121 | 17287 | 51075 | 51091 | 31034);
        let kind = match e.kind.as_ref() {
            ErrorKind::ServerSelection { .. } | ErrorKind::ConnectionPoolCleared { .. } => {
                ApiErrorKind::Unavailable
            }
            ErrorKind::Command(ce) if ce.code == 11000 => ApiErrorKind::Conflict,
            ErrorKind::Command(ce) if is_client_command(ce.code) => ApiErrorKind::BadRequest,
            ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we)) if we.code == 11000 => {
                ApiErrorKind::Conflict
            }
            ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we))
                if is_client_command(we.code) =>
            {
                ApiErrorKind::BadRequest
            }
            // insert_many (ordered) surfaces write failures as
            // ErrorKind::InsertMany with per-document codes, not WriteError
            ErrorKind::InsertMany(ime) => {
                let codes: Vec<i32> = ime
                    .write_errors
                    .as_ref()
                    .map(|wes| wes.iter().map(|w| w.code).collect())
                    .unwrap_or_default();
                if codes.iter().any(|&c| c == 11000) {
                    ApiErrorKind::Conflict
                } else if codes.iter().any(|&c| is_client_command(c)) {
                    ApiErrorKind::BadRequest
                } else {
                    ApiErrorKind::Internal
                }
            }
            // bulk_write (upsert-many on PATCH) surfaces write failures the
            // same way: ErrorKind::BulkWrite with per-index codes
            ErrorKind::BulkWrite(bwe) => {
                let codes: Vec<i32> = bwe.write_errors.values().map(|w| w.code).collect();
                if codes.iter().any(|&c| c == 11000) {
                    ApiErrorKind::Conflict
                } else if codes.iter().any(|&c| is_client_command(c)) {
                    ApiErrorKind::BadRequest
                } else {
                    ApiErrorKind::Internal
                }
            }
            _ => ApiErrorKind::Internal,
        };
        let msg = match kind {
            // keep connectivity detail (hosts/IPs are scrubbed by sanitize),
            // but never leak raw driver/server internals to clients
            ApiErrorKind::Unavailable => {
                format!("database operation failed: {}", sanitize(&e.to_string()))
            }
            ApiErrorKind::Conflict => "duplicate key error".to_string(),
            _ => "database operation failed".to_string(),
        };
        ApiError::new(kind, msg)
    }
}

impl From<bson::de::Error> for ApiError {
    fn from(e: bson::de::Error) -> Self {
        ApiError::new(
            ApiErrorKind::BadRequest,
            format!("invalid document: {}", sanitize(&e.to_string())),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_paths() {
        assert_eq!(
            sanitize("failed at /etc/hostname: 1"),
            "failed at <path> 1" // ':' is part of the path token
        );
        assert_eq!(sanitize("read \\\\server\\share\\f.txt"), "read <path>");
        assert_eq!(sanitize("open C:\\Users\\x\\f.txt"), "open <path>");
        assert_eq!(sanitize("go C:/Users/x/f.txt"), "go <path>");
        assert_eq!(sanitize("url http://localhost:27017/x"), "url htt<path>");
        assert_eq!(sanitize("no paths here"), "no paths here");
    }

    #[test]
    fn sanitize_ips() {
        assert_eq!(sanitize("peer 192.168.1.1:8000"), "peer <ip>:8000");
        assert_eq!(sanitize("v6 [::1]:27017"), "v6 <ip>:27017");
        assert_eq!(sanitize("v6b fe80::1"), "v6b <ip>");
        assert_eq!(sanitize("v6c ::ffff:192.0.2.1"), "v6c <ip>");
        // hostnames and host:port are deployment config, NOT scrubbed
        assert_eq!(sanitize("host localhost:27017"), "host localhost:27017");
        assert_eq!(sanitize("v 1.2.3"), "v 1.2.3");
        assert_eq!(sanitize("hex deadbeef:1"), "hex deadbeef:1");
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        ApiError::new(
            ApiErrorKind::BadRequest,
            format!("invalid JSON: {}", sanitize(&e.to_string())),
        )
    }
}

// ---------------------------------------------------------------------------
// Contract-preserving extractors
//
// axum's built-in `Json`/`Query` rejections bypass the handler's error type
// entirely (the Handler impl calls `Rejection::into_response()` directly,
// never `From<Rejection> for E`), so they leak plain-text bodies and odd
// status codes (e.g. 422 for missing fields). These wrappers map every
// extraction failure onto the standard {error, code, status} contract.
// ---------------------------------------------------------------------------

/// JSON request body with the standard error contract on extraction failure.
/// Use like `Json`: `JsonBody(body): JsonBody<SomeBody>`.
pub struct JsonBody<T>(pub T);

impl<S, T> FromRequest<S> for JsonBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(
        req: axum::http::Request<axum::body::Body>,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let axum::Json(v) = Json::<T>::from_request(req, state).await.map_err(|e| {
            ApiError::bad_request(format!(
                "invalid request body: {}",
                sanitize(&e.to_string())
            ))
        })?;
        Ok(JsonBody(v))
    }
}

/// Query-string extractor with the standard error contract on extraction
/// failure. Use like `Query`: `QueryBody(params): QueryBody<SomeParams>`.
pub struct QueryBody<T>(pub T);

impl<S, T> FromRequestParts<S> for QueryBody<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Query(v) = Query::<T>::from_request_parts(parts, state)
            .await
            .map_err(|e| {
                ApiError::bad_request(format!(
                    "invalid query string: {}",
                    sanitize(&e.to_string())
                ))
            })?;
        Ok(QueryBody(v))
    }
}
