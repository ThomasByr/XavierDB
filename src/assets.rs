//! Dashboard static assets, embedded at compile time.

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

const INDEX: &str = include_str!("assets/index.html");
const STYLES: &str = include_str!("assets/styles.css");
const APP_JS: &str = include_str!("assets/app.js");
const FAVICON: &[u8] = include_bytes!("assets/logo.png");

fn headers_for(ext: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    let ct = match ext {
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        _ => "text/html; charset=utf-8",
    };
    h.insert(header::CONTENT_TYPE, ct.parse().unwrap());
    h.insert(header::CACHE_CONTROL, "no-cache".parse().unwrap());
    h
}

pub async fn dashboard_index() -> Response {
    (headers_for("html"), INDEX).into_response()
}

pub async fn dashboard_assets(axum::extract::Path(rest): axum::extract::Path<String>) -> Response {
    match rest.as_str() {
        "styles.css" => (headers_for("css"), STYLES).into_response(),
        "app.js" => (headers_for("js"), APP_JS).into_response(),
        "logo.png" => (headers_for("png"), FAVICON).into_response(),
        _ => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

pub async fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"not found","code":"NOT_FOUND","status":404}"#,
    )
        .into_response()
}
