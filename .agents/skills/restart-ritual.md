# Server restart ritual — READ BEFORE ANY REBUILD

**Some OSes refuse to overwrite a running executable** — `cargo build` fails
until the server is killed (on the dev machine the error is "Accès refusé").

Each step a SEPARATE shell command; the start must be its own command — a
shell that times out can kill the disowned server: keep commands short and
use `--max-time` on curls. (`A && B &` backgrounds the whole chain — run
build and server-start as SEPARATE commands.)

1. stop the server by process name — POSIX: `pkill XavierDB`; Windows:
   `taskkill //F //IM XavierDB.exe` (plain `taskkill /F /IM XavierDB.exe`
   in non-MSYS shells). Verify down (`curl /health` fails).
2. `cargo build --tests` (rebuilds the server binary AND keeps the
   test-mode fingerprints fresh)
3. start detached (own command), e.g.
   `./target/debug/XavierDB >> /tmp/xdb.log 2>&1 & disown`
   (the binary is `XavierDB.exe` on Windows).

## TRAP — the clean-sequence rule (single most load-bearing build fact)

**Never run plain `cargo build` between `cargo build --tests` and
`cargo test`.** mongodb is compiled as TWO separate units (normal graph vs
test graph, different rlib hashes); the normal-mode build re-invalidates the
test-mode bin fingerprint ("Dirty: info of dependency mongodb changed") and
`cargo test` then tries to relink the server binary → lock. `cargo test
--no-run` has the same effect. A full `cargo clean` does NOT cure it.

The only clean sequence is: **kill → `cargo build --tests` → start →
`cargo test`**. `cargo check` is safe while the server runs.

## When src/ changed (full test cycle)

`cargo test` relinks the bin test harness whenever src/ or Cargo.toml
changed — same lock hazard. Ritual:

1. kill the server (above) → verify down
2. `cargo build --tests`  ← the ONLY build needed; leaves the runnable
   server binary in place
3. start the server (own command)
4. `cargo test` (or `XDB_TEST_MONGO_URI=mongodb://127.0.0.1:27017 cargo test`
   for the env-gated equivalence test)

Incremental test runs (no src/ change) don't relink — `cargo test` alone is
fine.

## Second server instance

A second instance can run with env overrides (e.g. `PORT=8443`) sharing the
same cwd state files — fine for read-only testing; stop it by PID
(`lsof -i :8443` on POSIX, `netstat -ano | grep :8443` on Windows), never by
process name (that kills every instance).
