# Architecture — Auth

_Split from the former `knowledge/architecture.md` (2026-08-24); section map in `knowledge/architecture/README.md`._

## 1. Auth

- `POST /auth` `{identifier, token}` → validates identifier against
  `authorized_keys.yml` + Argon2id-verifies the shared token → returns
  `{token, token_type:"Bearer", expires_in:5400, identifier}` + `Set-Cookie:
  xdb_token` (HttpOnly; Secure under TLS). 401 bad creds, 403 BLOCKED, 429
  throttle.
- JWT: HS256, secret = server.yml `auth.jwt_secret` or random-per-start; lifetime from
  `config.global.jwt_token_lifetime_minutes` (default 90). Expired/malformed →
  401 with a 5s leeway; reason swallowed. Client loop: on 401 re-auth, on 403
  do NOT re-auth.
- Blocked ids (in `config.blocked`) → 403 BLOCKED at `/auth`.
- **The app token is shared by every name under an app** (one Argon2id hash
  per app in authorized_keys.yml): any holder can /auth as ANY `name@app` —
  existing or not (new names are auto-added to the yml on first login). The
  name_id is a permission-routing label, not a credential; name-level rules
  separate identities within the app only. Each name needs its own /auth for
  its own JWT (sub = exact name).
- Dashboard sessions: in-memory DashMap (`xdb_admin` cookie, Path=/dashboard,
  TTL `config.auth.session_ttl_hours` default 24) — **restart = re-login**.
- Login throttles: `/auth` and dashboard login have SEPARATE per-IP 1-minute
  windows. `/auth` always uses `config.auth.max_per_minute_per_ip` (default
  30, dashboard-editable); dashboard login uses server.yml
  `admin.max_logins_per_ip_per_minute` (default 5, clamped 1..=10_000).
  **Client IP source (2026-08-18):** socket peer by default; when server.yml
  `network.trust_proxy_headers` (env `TRUST_PROXY_HEADERS`, compose sets it
  true) is on, the proxy header wins — `X-Real-IP`, else the LAST
  `X-Forwarded-For` entry (the proxy-appended one; must parse as an IP or is
  ignored). Helpers `routes_q::{proxy_ip, effective_ip, effective_addr}`
  (unit-tested). Safe in the compose deployment because the port is
  published to 127.0.0.1 only (nginx is the sole connector, sets X-Real-IP).

### Auth Q&A (verified from code — auth.rs, perms.rs)

- **Why JSON↔BSON conversions at all?** The API is a REST/JSON facade over
  BSON-native MongoDB. `json_to_bson` (requests): extended-JSON tokens decoded
  (`$oid`, `$date`, `$numberLong/Int/Double/Decimal`, `$binary`, `$regex`+
  `$options`, `$timestamp`, `$minKey/$maxKey`); u64 > i64::MAX becomes
  Decimal128; `$where`/`$function` rejected (400). `bson_to_json` (responses):
  type-fidelity rules so re-inserting a response never silently changes types
  (ObjectId → hex, DateTime → ISO, NaN/±Inf → `{"$numberDouble":…}`,
  Decimal128 → `{"$numberDecimal":…}`); cursor page values additionally use
  canonical extended JSON. Without the fidelity rules, NaN read back as null
  and Decimal128 as a plain string (both were real bugs).
- **Can an authenticated client act as any name_id under its app?** The JWT
  is bound to the exact `sub` issued at /auth and signed with the server
  secret — name1's JWT cannot be re-claimed as name2, and per-request
  authorization uses the JWT's sub+app claims. BUT the app token is shared by
  every name under the app: anyone holding it can /auth as ANY `name@app`.
  Name rules separate identities *within* the app only — not a security
  boundary against token holders.
- **Does each name_id need its own /auth call?** Yes. A JWT is per-name;
  name-level rules apply per JWT. Token expiry is per-user.
- Verified mechanics: identifier = `name@app`, each part 1–64 chars of
  `[A-Za-z0-9-_.:~]`. /auth: throttle (per peer IP) → parse → check_block
  (403) → spawn_blocking Argon2id verify (dummy PHC for unknown apps, timing
  equalized) → auto-add name → sign JWT. Claims: sub, app, iat, exp, jti;
  5 s leeway. Blocked: `name@app` exact or bare `app` → 403 BLOCKED.

