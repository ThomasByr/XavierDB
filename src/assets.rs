//! Dashboard static assets, embedded at compile time.

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

const INDEX: &str = include_str!("assets/index.html");
const STYLES: &str = include_str!("assets/styles.css");
const APP_JS: &str = include_str!("assets/app.js");
const FAVICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect width="24" height="24" rx="6" fill="#6d4aff"/><path d="M7 12h10M12 7v10" stroke="#fff" stroke-width="2.5" stroke-linecap="round"/></svg>"##;

fn headers_for(ext: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    let ct = match ext {
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
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
        "favicon.svg" => (headers_for("svg"), FAVICON).into_response(),
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
