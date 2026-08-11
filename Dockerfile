# syntax=docker/dockerfile:1
#
# XavierDB — builds the dashboard assets in a node stage, then compiles
# the Rust server in the official rust slim image and runs in that same
# image (user constraint: api image = rust:1-slim-bookworm).

# ---------- dashboard assets (esbuild) ----------
FROM node:22-bookworm-slim AS assets
WORKDIR /dashboard
# reproducible install from the lockfile; layer cached unless package.json changes
COPY package.json package-lock.json ./
RUN npm ci
COPY src/assets/ts ./src/assets/ts
# generates src/assets/app.js — the only generated asset (html/css are static)
RUN npm run build

# ---------- build + runtime ----------
FROM rust:1-slim-bookworm

# The rust slim image ships gcc + libc6-dev, but aws-lc-sys (the crypto
# backend pulled in by rustls) also needs cmake, perl and pkg-config.
# curl is for the compose healthcheck; ca-certificates for TLS roots.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        cmake \
        curl \
        perl \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependency compilation: compile every crate once against a dummy
# main, so the layer is reused unless Cargo.toml changes.
COPY Cargo.toml ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Build the real source; the freshly built dashboard asset from the node
# stage wins over any stale copy in the build context.
COPY . .
COPY --from=assets /dashboard/src/assets/app.js ./src/assets/app.js
RUN cargo build --release \
    && cp target/release/XavierDB /usr/local/bin/XavierDB

# Runtime working directory: .env, config and authorized_keys.yml are read
# and written here; compose mounts the repo root over /app, so the repo
# files are the container's state files.
WORKDIR /app

EXPOSE 8000

CMD ["XavierDB"]
