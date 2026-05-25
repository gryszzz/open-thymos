# Multi-stage build for the OpenThymos server.
#
# Stage 1 caches cargo's registry + built dependencies as a dedicated layer
# so that source-only edits don't invalidate third-party compilation.

FROM rust:1.90-slim AS builder

WORKDIR /build

RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

# Copy the workspace manifests first to prime the dependency cache.
COPY Cargo.toml Cargo.lock ./
COPY crates crates

# Real build. With BuildKit enabled this layer reuses the cached cargo
# registry and target directory whenever only source changes.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release -p thymos-server -p thymos-cli \
 && cp target/release/thymos-server /usr/local/bin/thymos-server \
 && cp target/release/thymos        /usr/local/bin/thymos

# Stage 2: minimal runtime.
# Use trixie (glibc 2.39) to match the rust:1.90-slim builder. Bookworm
# (glibc 2.36) cannot load binaries linked against 2.39.
FROM debian:trixie-slim

# OCI image labels. Dynamic values come from build args; CI passes them via
# docker/metadata-action. Local `docker build` falls back to the defaults.
ARG VERSION=dev
ARG REVISION=unknown
ARG CREATED=1970-01-01T00:00:00Z

LABEL org.opencontainers.image.title="OpenThymos Runtime" \
      org.opencontainers.image.description="Execution substrate for governed machine cognition. Separates cognition from execution through the typed Intent -> Proposal -> Commit protocol." \
      org.opencontainers.image.url="https://github.com/gryszzz/OpenThymos" \
      org.opencontainers.image.source="https://github.com/gryszzz/OpenThymos" \
      org.opencontainers.image.documentation="https://github.com/gryszzz/OpenThymos#readme" \
      org.opencontainers.image.vendor="Exponet Labs" \
      org.opencontainers.image.licenses="Apache-2.0" \
      org.opencontainers.image.base.name="docker.io/library/debian:trixie-slim" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}" \
      org.opencontainers.image.created="${CREATED}"

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/thymos-server /usr/local/bin/thymos-server
COPY --from=builder /usr/local/bin/thymos        /usr/local/bin/thymos

RUN mkdir -p /data

ENV THYMOS_DB_PATH=/data/thymos-runs.db \
    RUST_LOG=info

EXPOSE 3001

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
  CMD curl -fsS http://localhost:3001/health || exit 1

CMD ["thymos-server"]
