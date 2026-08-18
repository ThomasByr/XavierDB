# Overview — what XavierDB is

A small, fast HTTP server (Rust, axum 0.8, tokio, mongodb driver) that exposes
a **MongoDB database through a REST API**: per-client authentication (JWT),
granular permissions (`authorized_keys.yml`), adaptive per-app document
limits, a binary config file with undo/redo history, and an embedded
Material-3-ish admin dashboard SPA (no JS libraries, no external fonts).
Edition 2024. No Python/Node at runtime (Node only at build time for
the dashboard TypeScript). Cross-platform Rust; no OS-specific code at runtime.

Routes (top level, `src/main.rs`):

```
POST /auth                                 client login -> JWT (+ HttpOnly cookie)
GET|POST|PUT|PATCH|DELETE /q/<db>/<coll>   MongoDB proxy (JWT-protected)
GET|POST|DELETE /q/<db>/<coll>/indexes     index list / ensure (idempotent) / drop (INDEX action)
GET  /ls                                   list databases the caller may read (?db=<db> -> collections)
/dashboard/ + /dashboard/api/*             admin dashboard (login-protected SPA + JSON API)
GET  /health                               cached health document (public)
```
