# syntax=docker/dockerfile:1

# ---------- dashboard assets (esbuild) ----------
FROM node:22-bookworm-slim AS assets
WORKDIR /dashboard
COPY package.json package-lock.json ./
RUN npm ci
COPY src/assets/ts ./src/assets/ts
RUN npm run build

# ---------- build + runtime ----------
FROM rust:1-slim-bookworm

# Install build dependencies + healthcheck utils
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        cmake \
        curl \
        perl \
        pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Stage 1: dummy build to cache dependencies
# Note: COPY Cargo.lock to ensure reproducible dependency resolution
COPY Cargo.toml Cargo.lock ./
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,id=xavier-target,target=/build/target \
    mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Stage 2: real build with incremental compilation
COPY . .
COPY --from=assets /dashboard/src/assets/app.js ./src/assets/app.js
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,id=xavier-target,target=/build/target \
    cargo build --release \
    && cp target/release/XavierDB /usr/local/bin/XavierDB

# Runtime configuration
WORKDIR /app

EXPOSE 8000

CMD ["XavierDB"]
