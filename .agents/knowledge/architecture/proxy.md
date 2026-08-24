# Architecture — /q Proxy, indexes, projection, batch writes

_Split from the former `knowledge/architecture.md` (2026-08-24); section map in `knowledge/architecture/README.md`._

## 3. `/q/<db>/<coll>` proxy (routes_q.rs)

- GET: params `filter`/`sort`/`projection` = URL-encoded JSON (extended JSON ok), `limit`,
  `cursor`. `projection` = top-level include/exclude object (values 1/0/true/false;
  mixed styles rejected except `_id`; `_id:0` ok; `{}` no-op; dotted/`$` keys →
  400 INVALID_PROJECTION). Sort fields + `_id` are force-added to the Mongo
  projection and stripped from the output (union+strip, dbq.rs
  `projection_document`/`projection_strip_fields`/`bson_to_json_projected`) so
  keyset pagination and the array-sort guard keep working — cursors are
  projection-independent. Response `{documents:[…], next_cursor, has_more, truncated,
  limit_applied, count}`. Server caps `limit` at the app's adaptive limit.
  Find deadline (2026-08-18): the whole find is wrapped in
  `tokio::time::timeout` (dbq.rs `find_page`, inner `find_page_inner`) with
  server.yml `runtime.find_timeout_ms` (default 10 000 ms, 0 = disabled,
  nonzero clamped 100..=3 600 000; env override `FIND_TIMEOUT_MS`; logged at
  startup as `find_timeout=…ms`) — a runaway query (multiplanner blowup on an
  unindexed sort) fails with **504 TIMEOUT** instead of hanging until the HTTP
  caller disconnects (which is what produced the "Interrupted operation as
  its client disconnected" Mongo log noise on prod). Applies to GET /q only;
  writes/counts/`/ls` are not deadline-wrapped.
- POST: `{filter?, data}` — no filter = insert (201): `data` object →
  `insert_one` (`{inserted_count:1, inserted_id}`); `data` array → `insert_many`
  (`{inserted_count:n, inserted_ids:[…]}` in input order, cap
  `state.max_insert_batch` — server.yml `runtime.max_insert_batch`, default
  1000 (main.rs, routes_q.rs `MAX_INSERT_BATCH`), must be ≥ 1, also published
  top-level in
  `/health`; empty/
  non-object/dup-`_id`-within-batch → 400 with nothing inserted; dup against
  existing data → 409 with ordered semantics — docs before the dup remain).
  With filter = update (200 `{matched_count, modified_count}`).
  **`data` is auto-wrapped in `$set` server-side** (routes_q.rs:480) —
  clients send plain `data: {field: value}`, NOT `{$set:…}`. Batch inserts
  count their document count into per-client rate accounting (rps) — a
  1000-doc batch reads as 1000 work units (routes_q.rs `record_request`).
  **Upsert-many is on PATCH** (see below): POST batches are insert-only.
- PUT = update (404 if 0 matched). PATCH = upsert (200 updated / 201
  inserted) with `{filter, data: object}`; **upsert-many** with `{data:
  [objects]}` (no filter) — always 200 `{matched_count, modified_count,
  inserted_count, upserted_count, inserted_ids, upserted_ids}` (ids in input
  order). Per element: has `_id` → upserting UpdateOne on `{_id}` with $set
  merge (filter carries the `_id`); no `_id` → InsertOne. One PATCH authorize
  covers the whole batch. Same
  `batch_to_docs` validation as POST (empty/non-object/dup-`_id`-within/
  over-cap → 400, nothing applied), `{filter, data: array}` → 400 "batch
  upsert takes no filter"; conflicts (existing `_id`/unique index) → 409
  ordered semantics, docs before the failing element remain (dbq.rs
  `bulk_upsert` via driver `Client::bulk_write`, requires MongoDB 8.0+;
  batch size counts into rate accounting). DELETE `{filter}` → `{deleted_count}` (404 if 0).
- Cursor pagination: keyset, opaque base64url cursor
  `{v, db, coll, sort:[[field,dir]..], last:[canonical-extjson..]}` with `_id`
  tiebreaker; listing cursors use plain JSON-string values and require
  `last.len() == sort.len()` (dbq.rs:361 — a mismatch breaks decoding:
  "wrong listing cursor"). Mixed-type/missing sort fields paginate correctly:
  each keyset column gets a same-type `$gt/$lt` branch plus a `$type`
  bracket-fallback branch (null boundaries skip the `$gt/$lt` branch to avoid
  re-serving nulls; NaN boundaries continue with `$gte: -Inf` and a `{f: NaN}`
  branch — NaN sorts first ascending on MongoDB 8; a `$gt` there would miss
  the -Inf tie-group, caught by the equivalence test). The type-bracket
  branches are gated by `runtime.keyset_type_brackets` (server.yml,
  startup-only, 2026-08-20): `"all"` (default) keeps them on every column;
  `"id-only"` drops them for `_id` columns (prod verified every collection's
  `_id` is a single BSON type — all strings or all ObjectIds); `"off"` drops
  them everywhere. Rationale: `$type` cannot use index bounds, so with
  "all" a deep keyset page over an `_id`-sorted collection is a residual
  filter over a full `_id` index scan (prod: ~183k keys examined per 101-doc
  page, ~250 ms, the main CPU hog); with "id-only" page 2+ compiles to a
  bare `{_id: {$gt}}` — verified keysExamined == limit+1. A single-arm
  keyset `$or` and an empty user filter collapse away (no `$or`/`$and`
  wrappers) — planner-friendlier and readable in the profiler. ARRAY sort values are
  refused with 400 when a page needs continuation (element-wise array sort is
  not representable in a keyset cursor).
- Filter hardening: server-side script operators `$where`/`$function` are
  rejected (400 INVALID_FILTER) everywhere a filter is parsed (GET/POST/PUT/
  PATCH/DELETE). Mongo client-caused command errors (bad regex, malformed
  shapes, validation) map to 400, duplicate keys to 409 CONFLICT.
- Output forms: non-finite doubles → `{"$numberDouble":"NaN"}`, Decimal128 →
  `{"$numberDecimal":"…"}` (plain strings/`null` would silently change the
  type on re-insert). `$regex`+`$options` (two-key object) converts to a real
  regex; `$timestamp` requires non-negative `t`/`i`.

### Index endpoints (GET/POST/DELETE /q/{db}/{coll}/indexes)

- Perm model: GET lists indexes under the plain `GET` action (read access
  ⇒ seeing index names/keys); POST (ensure) and DELETE (drop) both require
  the dedicated `INDEX` action — one capability for index management, not
  split by verb (document-DELETE must not imply dropping a unique index,
  and INDEX must not imply document-delete).
- ensure = idempotent createIndex, decision table: no index on those keys →
  create → 201 `{created:true,name}` (name auto-generated when omitted,
  `field_1_dir_…` style); same key pattern (any name) + same options →
  200 `{created:false,name:existing}`; same name different keys → 409; same
  keys different options (unique/sparse/TTL/partial filter) → 409 (changing
  a TTL would need collMod — refused loudly, v1). Implemented by listing
  first, then create_index — the driver's create gives no created/existed
  info.
- Flat request body `{keys, name?, unique?, sparse?, expire_after_seconds?,
  partial_filter_expression?}`; keys validated server-side (each value 1/-1
  or an index-type string like "text"/"2dsphere"/"hashed" → 400 otherwise).
  `$where`/`$function` rejected in partial_filter_expression. Drop is
  by NAME only (what GET returns); `_id_` refused with 400, unknown name
  404 (pre-checked — the raw driver error would be a 500).
- listIndexes on a missing collection fails with Mongo code 26 → mapped to
  a clean 404. createIndexes client errors (codes 67/85/86/118) map to 409
  with the sanitized server message.
- Driver facts (mongodb 3.8): `mongodb::IndexModel` +
  `mongodb::options::IndexOptions` (the `mongodb::index` module is PRIVATE —
  only IndexModel is re-exported at the root; IndexOptions lives under
  `mongodb::options`); `expire_after` is `Option<Duration>` (seconds ×),
  `IndexOptions::default()` exists (non_exhaustive but Default);
  `list_indexes` streams `IndexModel` (options field is `Option<IndexOptions>`,
  name lives INSIDE options); `create_index` → `CreateIndexResult{index_name}`.

### Projection implementation map (v1, 2026-08-13)

- error.rs: `InvalidProjection` variant (code "INVALID_PROJECTION", 400).
- dbq.rs: `ProjectionStyle`/`Projection`, `parse_projection(&Value)`,
  `projection_document(...)`, `projection_strip_fields(...)`,
  `bson_to_json_projected(&Document, &BTreeSet)`; `find_page` takes
  `projection: Option<Document>`.
- routes_q.rs: `FindParams.projection: Option<String>`, `parse_projection_json`;
  output mapped through `bson_to_json_projected`.
- `{_id:0}` alone → Exclude style → no projection sent → all fields except
  `_id` (real Mongo semantics; previously collapsed to `{}` docs — FIXED
  2026-08-14). `{_id:1}` alone unchanged. Include-only projections strip `_id`
  unless explicitly requested (`{name:1}` → only `name`; `{name:1,_id:1}`
  keeps it) — v1 contract.
- Regression net: env-gated `keyset_pagination_equivalence` unit test, phase 3
  covers projection (coll3 × 2 sorts × 2 limits × none/include {b}/exclude
  {c,_id:0}) — `_id` walk must equal the full-scan baseline AND visible fields
  == source ∩ expected exactly. It caught a wrong `$gt`→`$gte` mistake
  mid-implementation.

### Batch write driver facts (mongodb 3.8 — hard-won, battery-verified)

- `insert_many` write failures arrive as `ErrorKind::InsertMany(InsertManyError)`
  — NOT `ErrorKind::Write(WriteFailure::WriteError)`; error.rs needs a
  dedicated arm. `InsertManyResult.inserted_ids` is a `HashMap<usize, Bson>`
  (NOT BTreeMap) — dbq.rs sorts by index so `inserted_ids` keeps input order.
- `Client::bulk_write(models)` — `.verbose_results()` takes NO bool argument
  (type-level marker returning `BulkWrite<VerboseBulkWriteResult>`);
  `VerboseBulkWriteResult.insert_results: HashMap<usize, InsertOneResult>`
  (map `.inserted_id` yourself); `update_results` →
  `UpdateResult.upserted_id: Option<Bson>` (filter_map for upserted ids).
  Models: `mongodb::options::{WriteModel, UpdateOneModel, InsertOneModel}`,
  `mongodb::Namespace::new(db, coll)`; `.update()` takes
  `impl Into<UpdateModifications>` (From<Document> exists).
  `doc.remove("_id")` strips the id for the $set payload; the filter carries
  it and Mongo re-adds it on upsert-insert.
- Client error codes → 400: 2|9|14|121|17287|51075|51091|31034; any 11000 →
  409 CONFLICT; else 500.
- Validator collections via driver: `db.collection::<Document>(c).drop().await`
  (idempotent on missing) then `db.create_collection(c).validator(doc).await`.
- `count_documents(filter: Document)` (action API, returns u64).

