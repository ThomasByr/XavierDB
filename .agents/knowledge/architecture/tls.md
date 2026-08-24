# Architecture — TLS

_Split from the former `knowledge/architecture.md` (2026-08-24); section map in `knowledge/architecture/README.md`._

## 8. TLS (tls.rs)

- Optional TLS; BOTH cert and key are hot-reloaded. Verified live:
  matched-pair rotation reloads without restart (new CN served); key-file
  mismatch → warn + keep old; garbage cert → "no certificate found"
  fail-safe, listener unaffected by bad reloads.

