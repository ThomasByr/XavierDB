# API reference

Base URL: `http://<host>:<port>` (HTTPS when `TLS_CERT_PATH`/`TLS_KEY_PATH` are set).

Every error is JSON with a machine-readable code:

```json
{ "error": "human readable message", "code": "INVALID_FILTER", "status": 400 }
```

Exception: request bodies / query strings that fail **JSON parsing at the HTTP
layer** (malformed JSON body, `limit=abc`) are rejected by the framework with a
plain-text `400` and no JSON error body. A valid-JSON body missing required
fields (e.g. no `token`) gets a plain-text `422`. Valid JSON with all required
fields always yields the JSON error shape above.

| HTTP | code | meaning |
|---|---|---|
| 400 | `BAD_REQUEST` / `INVALID_FILTER` / `INVALID_SORT` / `INVALID_LIMIT` / `INVALID_CURSOR` / `INVALID_PROJECTION` | malformed input |
| 401 | `UNAUTHORIZED` | missing / invalid / expired JWT or dashboard session |
| 403 | `FORBIDDEN` | no permission for this operation |
| 403 | `BLOCKED` | the name_id or app_id is blocked (dashboard) |
| 404 | `NOT_FOUND` | resource not found (db doesn't exist; update/delete matched no document) |
| 409 | `CONFLICT` | duplicate key on insert |
| 429 | `TOO_MANY_REQUESTS` | auth brute-force throttle (per peer socket IP) |
| 500 | `INTERNAL_ERROR` | unexpected server error (generic message; details in the server log) |
| 503 | `UNAVAILABLE` | MongoDB unreachable |

---

## POST /auth — client login

```json
{ "identifier": "user1@provider1", "token": "the-app-shared-token" }
```

- `identifier` is `name_id@app_id`: both parts must be 1–64 chars of
  `[A-Za-z0-9-_.:~]` (`401` otherwise). Only the app credential is checked.
  The name is added to `authorized_keys.yml` the first time it is seen so it
  can be given its own rules later — note this auto-add rewrites the whole
  file (reformatted, comments lost), same as a dashboard save.
- The token is verified against the app's Argon2id hash — the only slow hash
  in the whole request path.
- Response `200`:
  ```json
  { "token": "eyJ...", "token_type": "Bearer", "expires_in": 5400,
    "identifier": "user1@provider1" }
  ```
  plus a `Set-Cookie: xdb_token=...` (HttpOnly, `Secure` under TLS).
- `401` on bad credentials, `403 BLOCKED` if the name or app is blocked,
  `429` beyond ~30 attempts/minute/IP (configurable). The throttle key is the
  peer socket IP — `X-Forwarded-For` is **not** trusted.

## GET /ls — list what the caller may read

Lists the databases the caller may GET; with `?db=X`, lists the collections
inside one database. The JWT only proves identity — the list is filtered live
against the caller's effective rules.

- `GET /ls` → `200`:
  ```json
  { "databases": ["db1", "db2"], "next_cursor": "…", "has_more": false,
    "limit_applied": 10 }
  ```
  Query params: `limit`, `cursor` (keyset pagination as `/q/`). Note:
  `/ls` reports `limit_applied` = the actual page length, while `/q` reports
  `min(requested, enforced)` — same field name, different meaning.
- `GET /ls?db=db1` → `200`:
  ```json
  { "db": "db1", "collections": ["items", "logs"] }
  ```
- `404 NOT_FOUND` when the database does not exist.
- `403 FORBIDDEN` when it exists but the caller may not access it.

## /q/ — the MongoDB proxy

Authentication: `Authorization: Bearer <jwt>` **or** the `xdb_token` cookie.
The JWT only proves identity; permissions are re-checked live on every request
against the current `authorized_keys.yml`.

### GET /q/{db}/{coll}

| param | meaning |
|---|---|
| `filter` | URL-encoded JSON, MongoDB filter syntax, e.g. `{"status":"active","n":{"$gt":5}}`. Extended JSON is accepted (`{"_id":{"$oid":"665f…"}}`, `$date`, `$numberLong`, `$regex` (+ optional `$options`), `$timestamp`, …). Server-side script operators (`$where`, `$function`) are **rejected** (400 `INVALID_FILTER`) — they execute JavaScript on the database server. |
| `sort` | URL-encoded JSON `{"field":1,"other":-1}` (1 asc, -1 desc). `_id` is appended automatically as tiebreaker. |
| `projection` | URL-encoded JSON object of top-level fields, e.g. `{"name":1}` (include) or `{"secret":0}` (exclude). Values must be `1`/`0`/`true`/`false`; mixing include and exclude is rejected except for `_id` (400 `INVALID_PROJECTION`). `_id:0` is allowed; an empty `{}` is a no-op. |
| `limit` | positive integer. If omitted, the server applies its adaptive limit. |
| `cursor` | opaque token from a previous response. Tampered cursors (wrong collection, wrong sort, unparseable page values) are rejected with `INVALID_CURSOR`. |

The server caps `limit` at the **adaptive limit** of the caller's app
(dashboard → Rate limit). When the cap bites, `truncated` is `true` and a
`next_cursor` is returned so the client can keep iterating page by page.

`projection` selects a subset of fields per document. The response documents
contain only the requested fields when present (documents lacking a field
simply omit it — `{}` is a valid projected document). Sort fields and `_id`
are always kept internally for keyset pagination and stripped from the
output unless requested, so `{"name":1}` works with any `sort` and with
`_id:0`. Dotted/nested paths (`{"a.b":1}`) and `$`-operator values
(`$meta`, `$slice`, `$elemMatch`) are rejected (400 `INVALID_PROJECTION`) —
top-level fields only. Cursors are projection-independent: reusing a cursor
with a different `projection` is safe.

Cursors are keyset-based (no `skip`), so pagination stays fast and consistent
even while documents change. Keyset pagination handles mixed-type and
missing sort fields correctly (null/type boundaries are continued via `$type`
bracket fallbacks; NaN/±Inf sort values are handled explicitly).
**Limitation:** if a page that needs a continuation contains a sort-field value
that is an **array**, the request fails with `400 BAD_REQUEST` — MongoDB sorts
arrays element-wise, which a value-based keyset cannot represent (silent data
loss or infinite loops otherwise). Use a different sort or filter. Values with
no plain JSON form are emitted as relaxed extended JSON
(`{"$numberDouble":"NaN"}`, `{"$numberDecimal":"…"}`) so responses
round-trip exactly.

`truncated` is set whenever the **requested** limit exceeds the enforced cap.
A request **without** `limit` counts as unbounded, so `truncated` is always
`true` for it — even when the result set happens to be complete
(`has_more: false`). Follow `has_more` for pagination, not `truncated`.

Response `200`:

```json
{ "documents": [ { "_id": "665f…", "n": 1 } ],
  "next_cursor": "eyJ…", "has_more": true,
  "truncated": true, "limit_applied": 25, "count": 25 }
```

### POST /q/{db}/{coll}

Body: `{ "filter"?: object, "data": object | object[] }`

- without `filter` → **insert** → `201 Created`:
  - `data` = **object** → single insert → `{ "inserted_count": 1, "inserted_id": "…" }`
    (`inserted_id` is the real stored `_id`; non-ObjectId ids are echoed in
    their plain JSON form)
  - `data` = **array** → batch insert → `{ "inserted_count": n, "inserted_ids": ["…", …] }`
    (`inserted_ids` in input order; docs without `_id` get a generated ObjectId)
- with `filter` → **update all matching** (`$set` data) → `200` `{ "matched_count": n, "modified_count": n }`

Batch insert (`data` as array) rules:
- the array must be non-empty, every element must be a JSON object, and the
  batch is capped at **`MAX_INSERT_BATCH`** (default 1000 documents,
  configurable via the `.env` — see CONFIGURATION.md) — violations return
  `400` `BAD_REQUEST` with **nothing inserted**.
- a `_id` duplicated *within* the batch is rejected up front with `400`
  `BAD_REQUEST` (no partial write).
- a `_id` that already exists in the collection returns `409 CONFLICT` with
  MongoDB ordered semantics: the insert aborts at the first duplicate and
  documents *before* it remain inserted.

> **Upsert-many lives on PATCH** (see below): POST batches are insert-only.

### PUT /q/{db}/{coll}

Body: `{ "filter": object, "data": object }` — update all matching (`$set`).
`200` with counts, `404` when nothing matched (no upsert).

### PATCH /q/{db}/{coll}

Two modes — the single-document upsert and the batch upsert-many:

- Body `{ "filter": object, "data": object }` — **single-document upsert**
  (`$set` merge). `200` when updated, `201` when a document was inserted
  (`upserted_id` present).
- Body `{ "data": [ object, … ] }` (array, **no** filter) — **upsert-many**,
  always `200`:

  ```json
  { "matched_count": n, "modified_count": n, "inserted_count": n,
    "upserted_count": n, "inserted_ids": ["…", …], "upserted_ids": ["…", …] }
  ```

  Per element: has `_id` → upsert by `{_id}` with a `$set` merge (the `_id`
  is carried by the filter, not the payload); no `_id` → plain insert with a
  generated ObjectId. Ids in both arrays are in **input order**. Batch
  validation matches POST batch insert (non-empty, objects only,
  `MAX_INSERT_BATCH` cap, no `_id` duplicated within the batch → `400` with
  nothing written). Conflicts against existing data (`_id` or a unique
  index) return `409` with MongoDB ordered semantics: the bulk aborts at
  the first failing element and elements before it remain applied. A
  `{filter, data: array}` body is rejected with `400` ("batch upsert takes
  no filter"); `{data: object}` without filter stays `400`. Requires
  MongoDB 8.0+ (uses the new `bulkWrite` command).

### DELETE /q/{db}/{coll}

Body: `{ "filter": object }` — delete all matching. `200`
`{ "deleted_count": n }`, `404` when nothing matched.

### Notes

- Permission actions map 1:1 to HTTP methods; a request needs
  `action` on `db.coll` for the caller's effective rules
  (see `authorized_keys.yml.example` for the resolution order).
- Updates auto-wrap `data` in MongoDB `$set` (see above); pure inserts
  store the documents verbatim (extended-JSON tokens like `$oid`, `$date`,
  `$numberDecimal` are converted server-side).
- Error messages are sanitized (paths, IPv4/IPv6 removed) before leaving the
  server; internal database failures return a generic message (details only
  in the server log). Bare hostnames / `host:port` pairs are not scrubbed.
  Client-caused database errors (bad regex, malformed shapes, validation
  failures) return `400`; duplicate keys return `409 CONFLICT`.

## GET /health — public, unauthenticated

Cached document, refreshed every `health.cache_ttl_seconds` (default 5 s)
in the background — spamming it costs nothing.

```json
{ "status": "ok",                     // ok | degraded | unhealthy
  "checked_at_ms": 1786448767863,
  "next_refresh_seconds": 5,
  "compute_latency_ms": 2.3,          // p50 server-side processing time (no network)
  "qps": 8.2,
  "max_insert_batch": 1000,           // insert-batch cap (MAX_INSERT_BATCH env; static per process)
  "app": { "status": "ok", "uptime_s": 25, "p50_latency_ms": 2.3,
           "total_requests": 1000, "active_cursors": 3 },
  "mongodb": { "reachable": true, "ping_latency_ms": 1.4, "error": null } }
```

`200` when `status == "ok"`, `503` otherwise. Nothing sensitive is exposed.

## Dashboard API

The `/dashboard/api/*` endpoints are documented in the
[admin guide](ADMIN_GUIDE.md#dashboard-api). They are session-cookie
protected and used by the dashboard itself — the only client-relevant
entry point is `POST /dashboard/api/login` (`{ "username", "password" }`).

---

## Working examples

Both examples cover the same real-world flow: health check → auth → dataset
listing → filtered/sorted query → insert / update / PUT / PATCH-upsert /
DELETE → cursor exhaustion → error handling. They run against any XavierDB
instance (`XDB_BASE` overrides the URL).

The credential in the examples only grants `GET` on `db1` in the stock
`authorized_keys.yml`, so the write calls return `403 FORBIDDEN` — handled and
reported, not fatal. Grant the credential write actions on the collection
(dashboard → Permissions) to see the writes succeed.

<details>
<summary><b>JavaScript</b> — Node ≥ 18, zero dependencies</summary>

```js
// xavierdb-example.js — a real-world XavierDB client (Node >= 18, zero deps).
// Run:  node xavierdb-example.js
// Env overrides: XDB_BASE, XDB_IDENTIFIER, XDB_TOKEN
//
// Covers: health check, auth, dataset listing, queries with filter/sort/limit,
// insert / update / PUT / PATCH-upsert / DELETE, cursor exhaustion, and simple
// error handling (401 / 403 / 404 / 503).
//
// Permission note: the credential used below only grants GET on db1 in the
// example authorized_keys.yml, so the write calls fail with 403 FORBIDDEN on
// a stock install. That is intentional — it exercises the error path. With a
// credential that has write permission on db1, the same calls succeed.

const BASE = process.env.XDB_BASE || "http://127.0.0.1:8000";
const IDENTIFIER = process.env.XDB_IDENTIFIER || "user1@provider1";
const APP_TOKEN = process.env.XDB_TOKEN || "my-secret-app-token";

/** Tiny HTTP helper: never throws on non-2xx; returns { status, ok, data }. */
async function api(path, { method = "GET", body, bearer } = {}) {
  const headers = { "Content-Type": "application/json" };
  if (bearer) headers.Authorization = `Bearer ${bearer}`;
  const res = await fetch(BASE + path, {
    method,
    headers,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const data = await res.json().catch(() => null);
  return { status: res.status, ok: res.ok, data };
}

/** Friendly message for the machine codes documented in API_REFERENCE.md. */
function friendly(status, code) {
  const map = {
    UNAUTHORIZED: "not authenticated — log in again",
    FORBIDDEN: "no permission for this operation",
    BLOCKED: "this name or app is blocked",
    NOT_FOUND: "no document matched the filter",
    TOO_MANY_REQUESTS: "rate limited — back off and retry",
    UNAVAILABLE: "MongoDB is unreachable right now",
  };
  return map[code] || `HTTP ${status}`;
}

function logWrite(name, res, describe) {
  if (res.ok) console.log(`  ${name}: ${describe()}`);
  else console.warn(`  ${name}: ${friendly(res.status, res.data?.code)} (${res.data?.code})`);
}

async function main() {
  // 1. Health check — public, no auth needed.
  const health = await api("/health");
  if (health.ok) {
    console.log(`health: ${health.data.status} (mongo ${health.data.mongodb.reachable ? "up" : "down"})`);
  } else {
    // 503 UNAVAILABLE: MongoDB unreachable — fail fast, retry with backoff later.
    console.warn(`health degraded (${health.status}): retry later`);
  }

  // 2. Auth — exchange the app credentials for a short-lived JWT.
  const auth = await api("/auth", {
    method: "POST",
    body: { identifier: IDENTIFIER, token: APP_TOKEN },
  });
  if (!auth.ok) throw new Error(`auth failed: ${friendly(auth.status, auth.data?.code)}`);
  const bearer = auth.data.token;
  console.log(`authenticated as ${auth.data.identifier} (expires_in ${auth.data.expires_in}s)`);

  // 3. List the databases this identity may read, then db1's collections.
  const ds = await api("/ls", { bearer });
  console.log(`databases: ${ds.data.databases.join(", ")}`);
  const colls = await api("/ls?db=db1", { bearer });
  console.log(`db1 collections: ${colls.data.collections.join(", ")}`);

  // 4. Query with a filter, sort and limit (URL-encoded JSON params).
  const page = await api(
    "/q/db1/items" +
      "?filter=" + encodeURIComponent(JSON.stringify({ n: { $gte: 2 } })) +
      "&sort=" + encodeURIComponent(JSON.stringify({ n: -1 })) +
      "&limit=2",
    { bearer }
  );
  console.log(`query: ${page.data.count} doc(s), truncated=${page.data.truncated}, limit_applied=${page.data.limit_applied}`);
  for (const doc of page.data.documents) console.log(`  item n=${doc.n}`);

  // 4b. Projection: request only the fields you need (top-level fields only).
  const names = await api(
    "/q/db1/items?limit=5&projection=" + encodeURIComponent(JSON.stringify({ name: 1 })),
    { bearer }
  );
  for (const doc of names.data.documents)
    console.log(`  name=${doc.name} (fields: ${Object.keys(doc).join(",")})`);

  // 5. Write operations. data is applied as MongoDB $set by the server.
  //    With a read-only credential each call returns 403 FORBIDDEN and is
  //    reported without crashing; with write permission they all succeed.
  const insert = await api("/q/db1/items", {
    method: "POST",
    body: { data: { n: 100, tag: "example" } },
    bearer,
  });
  logWrite("insert", insert, () => `inserted ${insert.data.inserted_count} (${insert.data.inserted_id})`);

  const update = await api("/q/db1/items", {
    method: "POST",
    body: { filter: { tag: "example" }, data: { n: 101 } },
    bearer,
  });
  logWrite("update", update, () => `matched ${update.data.matched_count}, modified ${update.data.modified_count}`);

  const put = await api("/q/db1/items", {
    method: "PUT",
    body: { filter: { n: 999999 }, data: { tag: "nope" } },
    bearer,
  });
  logWrite("put (no match)", put, () => `matched ${put.data.matched_count}`); // 404 NOT_FOUND: PUT never upserts

  const patch = await api("/q/db1/items", {
    method: "PATCH",
    body: { filter: { n: 102 }, data: { tag: "upserted" } },
    bearer,
  });
  logWrite("patch (upsert)", patch, () =>
    patch.data.upserted ? `inserted ${patch.data.upserted_id}` : `updated (matched ${patch.data.matched_count})`
  );

  const del = await api("/q/db1/items", {
    method: "DELETE",
    body: { filter: { tag: "example" } },
    bearer,
  });
  logWrite("delete", del, () => `deleted ${del.data.deleted_count}`);

  // 6. Cursor exhaustion — page through everything until has_more is false.
  //    The server caps each page at the app's adaptive limit (truncated=true);
  //    next_cursor is a keyset token, so paging stays O(1) per page.
  let cursor = null;
  let pages = 0;
  let total = 0;
  let cap = 0;
  do {
    const url = "/q/db1/items?limit=2" + (cursor ? "&cursor=" + encodeURIComponent(cursor) : "");
    const c = await api(url, { bearer });
    total += c.data.count;
    pages++;
    cap = c.data.limit_applied;
    cursor = c.data.has_more ? c.data.next_cursor : null;
  } while (cursor);
  console.log(`cursor exhausted: ${total} docs in ${pages} page(s) at limit ${cap}`);

  // 7. Simple error handling — a bad token produces a clean 401 UNAUTHORIZED.
  const bad = await api("/q/db1/items", { bearer: "garbage" });
  console.log(`bad token: ${friendly(bad.status, bad.data?.code)} (${bad.data?.code})`);
}

main().catch((e) => {
  console.error("fatal:", e.message);
  process.exit(1);
});
```

</details>

<details>
<summary><b>Python 3</b> — stdlib only, no pip install needed</summary>

```python
#!/usr/bin/env python3
# xavierdb_example.py - a real-world XavierDB client.
# Python 3 stdlib only (no pip install needed); works on Linux/macOS/Windows.
# Run:  python3 xavierdb_example.py
# Env overrides: XDB_BASE, XDB_IDENTIFIER, XDB_TOKEN
#
# Covers: health check, auth, dataset listing, queries with filter/sort/limit,
# insert / update / PUT / PATCH-upsert / DELETE, cursor exhaustion, and simple
# error handling (401 / 403 / 404 / 503).
#
# Permission note: the credential used below only grants GET on db1 in the
# example authorized_keys.yml, so the write calls fail with 403 FORBIDDEN on
# a stock install. That is intentional - it exercises the error path. With a
# credential that has write permission on db1, the same calls succeed.

import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request

BASE = os.environ.get("XDB_BASE", "http://127.0.0.1:8000")
IDENTIFIER = os.environ.get("XDB_IDENTIFIER", "user1@provider1")
APP_TOKEN = os.environ.get("XDB_TOKEN", "my-secret-app-token")


def api(path, method="GET", body=None, bearer=None):
    """Send a request. Never raises on 4xx/5xx: returns (status, parsed)."""
    headers = {"Content-Type": "application/json"}
    if bearer:
        headers["Authorization"] = "Bearer " + bearer
    data = json.dumps(body).encode("utf-8") if body is not None else None
    req = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            payload = resp.read()
            return resp.status, (json.loads(payload) if payload else {})
    except urllib.error.HTTPError as e:  # 4xx/5xx carry the XavierDB error doc
        payload = e.read()
        try:
            return e.code, json.loads(payload)
        except ValueError:
            return e.code, {}


def friendly(status, code):
    """Friendly message for the machine codes documented in API_REFERENCE.md."""
    return {
        "UNAUTHORIZED": "not authenticated - log in again",
        "FORBIDDEN": "no permission for this operation",
        "BLOCKED": "this name or app is blocked",
        "NOT_FOUND": "no document matched the filter",
        "TOO_MANY_REQUESTS": "rate limited - back off and retry",
        "UNAVAILABLE": "MongoDB is unreachable right now",
    }.get(code, "HTTP %d" % status)


def log_write(name, status, res, describe):
    if 200 <= status < 300:
        print("  %s: %s" % (name, describe()))
    else:
        print("  %s: %s (%s)" % (name, friendly(status, res.get("code")), res.get("code")), file=sys.stderr)


def main():
    # 1. Health check - public, no auth needed.
    status, health = api("/health")
    if status == 200:
        print("health: %s (mongo %s)" % (health["status"], "up" if health["mongodb"]["reachable"] else "down"))
    else:
        # 503 UNAVAILABLE: MongoDB unreachable - fail fast, retry with backoff.
        print("health degraded (HTTP %d): retry later" % status, file=sys.stderr)
        return 1

    # 2. Auth - exchange the app credentials for a short-lived JWT.
    status, auth = api("/auth", method="POST", body={"identifier": IDENTIFIER, "token": APP_TOKEN})
    if status != 200:
        raise SystemExit("auth failed: " + friendly(status, auth.get("code")))
    bearer = auth["token"]
    print("authenticated as %s (expires_in %ss)" % (auth["identifier"], auth["expires_in"]))

    # 3. List the databases this identity may read, then db1's collections.
    _, ds = api("/ls", bearer=bearer)
    print("databases: %s" % ", ".join(ds["databases"]))
    _, colls = api("/ls?db=db1", bearer=bearer)
    print("db1 collections: %s" % ", ".join(colls["collections"]))

    # 4. Query with a filter, sort and limit (URL-encoded JSON params).
    query = (
        "/q/db1/items"
        + "?filter=" + urllib.parse.quote(json.dumps({"n": {"$gte": 2}}), safe="")
        + "&sort=" + urllib.parse.quote(json.dumps({"n": -1}), safe="")
        + "&limit=2"
    )
    _, page = api(query, bearer=bearer)
    print("query: %d doc(s), truncated=%s, limit_applied=%d" % (page["count"], page["truncated"], page["limit_applied"]))
    for doc in page["documents"]:
        print("  item n=%s" % doc["n"])

    # 4b. Projection: request only the fields you need (top-level fields only).
    proj_query = "/q/db1/items?limit=5&projection=" + urllib.parse.quote(json.dumps({"name": 1}), safe="")
    _, names = api(proj_query, bearer=bearer)
    for doc in names["documents"]:
        print("  name=%s (fields: %s)" % (doc.get("name"), ",".join(sorted(doc))))

    # 5. Write operations. data is applied as MongoDB $set by the server.
    #    With a read-only credential each call returns 403 FORBIDDEN and is
    #    reported without crashing; with write permission they all succeed.
    status, res = api("/q/db1/items", method="POST", body={"data": {"n": 100, "tag": "example"}}, bearer=bearer)
    log_write("insert", status, res, lambda: "inserted %d (%s)" % (res["inserted_count"], res["inserted_id"]))

    status, res = api("/q/db1/items", method="POST",
                      body={"filter": {"tag": "example"}, "data": {"n": 101}}, bearer=bearer)
    log_write("update", status, res, lambda: "matched %d, modified %d" % (res["matched_count"], res["modified_count"]))

    status, res = api("/q/db1/items", method="PUT",
                      body={"filter": {"n": 999999}, "data": {"tag": "nope"}}, bearer=bearer)
    log_write("put (no match)", status, res, lambda: "matched %d" % res["matched_count"])  # 404: PUT never upserts

    status, res = api("/q/db1/items", method="PATCH",
                      body={"filter": {"n": 102}, "data": {"tag": "upserted"}}, bearer=bearer)
    log_write("patch (upsert)", status, res,
              lambda: ("inserted %s" % res["upserted_id"]) if res.get("upserted") else ("updated (matched %d)" % res["matched_count"]))

    status, res = api("/q/db1/items", method="DELETE", body={"filter": {"tag": "example"}}, bearer=bearer)
    log_write("delete", status, res, lambda: "deleted %d" % res["deleted_count"])

    # 6. Cursor exhaustion - page through everything until has_more is false.
    #    The server caps each page at the app's adaptive limit (truncated=true);
    #    next_cursor is a keyset token, so paging stays O(1) per page.
    cursor, pages, total, cap = None, 0, 0, 0
    while True:
        url = "/q/db1/items?limit=2"
        if cursor:
            url += "&cursor=" + urllib.parse.quote(cursor, safe="")
        _, c = api(url, bearer=bearer)
        total += c["count"]
        pages += 1
        cap = c["limit_applied"]
        if not c["has_more"]:
            break
        cursor = c["next_cursor"]
    print("cursor exhausted: %d docs in %d page(s) at limit %d" % (total, pages, cap))

    # 7. Simple error handling - a bad token produces a clean 401 UNAUTHORIZED.
    status, bad = api("/q/db1/items", bearer="garbage")
    print("bad token: %s (%s)" % (friendly(status, bad.get("code")), bad.get("code")))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as e:  # network down, timeout, malformed JSON, ...
        print("fatal: %s" % e, file=sys.stderr)
        sys.exit(1)
```

</details>
