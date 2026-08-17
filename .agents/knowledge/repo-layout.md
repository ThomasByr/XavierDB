# Repository layout

```
XavierDB/
├── README.md                    # quick start (Docker-first) + bare metal in <details>
├── LICENSE                      # MIT, Copyright (c) 2026 Thomas BOUYER
├── AGENTS.md                    # minimum instructions + pointers to .agents/
├── .agents/                     # agent knowledge + skills (this tree)
│   ├── knowledge/               #   reference facts (architecture, API contracts, ...)
│   └── skills/                  #   procedural how-tos (restart ritual, build, battery, ...)
├── docs/                        # the full documentation set
│   ├── ADMIN_GUIDE.md           #   dashboard views, ops, troubleshooting, sparse dashboard API section
│   ├── API_REFERENCE.md         #   client API only + verified JS/Python examples
│   └── CONFIGURATION.md         #   config file fields, adaptive-limit formula, perms format
├── compose.yaml                 # 2 services: xavierdb (MongoDB) + api; api mounts repo over /app
├── Dockerfile                   # node stage (esbuild) + single-stage rust:1-slim-bookworm build/run
├── .dockerignore                # full exclusions (dirs, state files, *.md/*.swp/*.tmp) — see skills/docker.md
├── .gitignore                   # /target, .env, server.yml, authorized_keys.yml, config, config.bak*, node_modules/
├── .github/workflows/            # CI/CD
│   └── deploy-site.yml           #   VitePress site -> GitHub Pages (xavierdb.fr) on web/** changes
├── web/                          # public marketing site — VitePress (its own npm project)
│   ├── package.json / package-lock.json  # vitepress only; scripts: docs:dev/build/preview
│   ├── .vitepress/config.mjs     #   title, base '/', cleanUrls, themeConfig.logo
│   ├── .vitepress/theme/         #   index.js (extends DefaultTheme) + custom.css (home green gradients)
│   ├── index.md                  #   home layout: hero title + logo + 2 buttons (GitHub, admin dashboard)
│   ├── public/                   #   copied verbatim to site root
│   │   ├── CNAME                 #     xavierdb.fr — GitHub Pages custom domain
│   │   └── logo.png              #     site logo (navbar + hero), served at /logo.png
│   └── .gitignore                #   node_modules/, .vitepress/dist/, .vitepress/cache/
├── Cargo.toml / Cargo.lock      # Rust workspace (axum, tokio, mongodb, rustls/aws-lc-sys, argon2, notify, serde_yaml…)
├── package.json / package-lock.json   # esbuild devDependency only (dashboard TS -> JS)
├── examples/                      # standalone crate: 8 runnable client examples (see examples/README.md)
│   ├── Cargo.toml / Cargo.lock    #   own deps (ureq + serde_json only), own lockfile
│   └── src/bin/                   #   per example: setup_<name>.rs (dashboard API) + <name>.rs (client API)
├── tests/                         # integration battery — BLACK-BOX HTTP tests, need a running server+Mongo
│   ├── common/mod.rs              #   shared helpers: fixture world docs, cached JWTs/admin cookie, HTTP wrappers, suite lock
│   ├── bootstrap.sh               #   one-time fixture bootstrap (idempotent; dashboard creds from env or credentials.md)
│   └── auth_flow.rs, crud_verbs.rs, dashboard_api.rs, edge_data.rs, meta_endpoints.rs,
│       multi_app.rs, pagination.rs, perms_matrix.rs, projection.rs, query_filters.rs,
│       smoke.rs, watcher_reload.rs   # 110 tests, ~30 s full run
├── server.yml.example          # documented settings template (copy to server.yml; embedded in the binary)
├── .env.example                # Docker-compose only: UID/GID (the app never reads .env)
├── authorized_keys.yml.example  # documented permissions template
├── src/
│   ├── main.rs                  # startup, server.yml settings, watchers (config/perms hot reload), router, /health
│   ├── auth.rs                  # JWT issue/verify, /auth, Argon2id, throttle
│   ├── settings.rs              # server.yml loader (env > file > default), clamps, password bootstrap
│   ├── routes_q.rs              # /q proxy + /ls handler, per-request perms check, cursor pagination
│   ├── dbq.rs                   # MongoDB queries, cursor encode/decode (keyset), listing cursors
│   ├── perms.rs                 # authorized_keys.yml parsing, globs, layered first-match-wins evaluation
│   ├── config.rs                # ConfigFile: XDB1 magic + crc32 + bincode, atomic writes, backups, history/undo
│   ├── routes_admin.rs          # all /dashboard/api/* endpoints (~681 lines)
│   ├── metrics.rs               # adaptive limit engine, rate/EMA computation, pressure
│   ├── state.rs                 # AppState, ClientStats (delta-based counters), sessions
│   ├── tls.rs                   # optional TLS, cert hot reload
│   ├── error.rs                 # {error, code, status} contract
│   └── assets/
│       ├── assets.rs            # serves embedded SPA files under /dashboard/ no-cache
│       ├── index.html           # static shell (login + app shell)  [static]
│       ├── styles.css           # design tokens + all styles              [static]
│       ├── app.js               # GENERATED by esbuild — never hand-edit
│       └── ts/app.ts            # dashboard SPA source (~2050 lines TS) — edit here
├── server.yml                  # local settings (gitignored); NOT in Docker image
├── .env                        # Docker-compose UID/GID only; gitignored; NOT in Docker image
├── config / config.bak*         # binary settings + backups; gitignored; runtime state
├── authorized_keys.yml          # app credentials + permissions; gitignored; runtime state
├── target/                      # build output (excluded everywhere)
└── node_modules/                # npm deps (excluded everywhere)
```

## What the image vs. the repo contains (Docker)

`.dockerignore` (full list in the file) excludes dirs (`target`,
`node_modules`, `examples/`, `web/`, `.agents/`…), state files (`server.yml`
— exact name only, `server.yml.example` MUST stay: include_str! needs it —,
`.env*`, `config`, `config.bak*`, `authorized_keys.yml*`) and `*.md` from
the build context. `COPY . .` (build stage) lands in `/build`; the
runtime stage's `/app` workdir is empty until compose mounts the repo root
over it (`.:/app`), so the repo files ARE the container's state files: the
container reads/writes the same `server.yml`/`config`/`config.bak`/`authorized_keys.yml` as
bare metal. Secrets never enter image layers (mount ≠ image). Mongo data stays
in a named volume (`xavier_mongo_db`, TEMPORARY on this Windows host where
`${HOME}` is undefined) — the intended bind mount is
`${HOME}/data/xavier-mongo-db`.
