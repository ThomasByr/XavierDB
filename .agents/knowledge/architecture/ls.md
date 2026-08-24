# Architecture — /ls

_Split from the former `knowledge/architecture.md` (2026-08-24); section map in `knowledge/architecture/README.md`._

## 4. `/ls` (replaced `/q/dataset` — no alias, that route now 404s; `dataset` is NOT reserved)

- `GET /ls` → `{databases: ["a","b"], next_cursor, has_more, limit_applied}` —
  FLAT name strings, permission-filtered, cursor-paginated over dbs only.
- `GET /ls?db=X` → `{db:"X", collections:[...]}`; 404 when X doesn't exist
  (checked vs `dbq::list_databases`); 403 when X exists but the caller has no
  access (`perms.listable_databases(...)` empty → 403). 401 only from
  `authenticate()`.
- Handler: `routes_q::list_visible`, registered on the top-level router.
- Listing cursor: `sort: [("name",1)]` ONLY — pagination is pure name-based
  `retain(|d| d > last)`; a previous build emitted sort with 2 entries but
  `last` with 1, and `Cursor::decode` requires `last.len() == sort.len()` →
  every second page failed "wrong listing cursor". Don't add a second sort
  column without a matching boundary value.

