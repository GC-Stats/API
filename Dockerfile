# GC-Stats — API
#
# Multi-stage Docker build: compiles the Rust binary and copies it into a
# slim runtime image.
#
# Copyright (c) 2026 Alice Alleman — GC-Stats-API
# License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
# Repository: https://github.com/GC-Stats/API

FROM rust:1.95-slim-bookworm AS builder

RUN apt-get update && apt-get install -y \
    pkg-config libssl-dev build-essential curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY .sqlx ./.sqlx
COPY swagger-ui-overrides ./swagger-ui-overrides

ENV SWAGGER_UI_OVERWRITE_FOLDER=/app/swagger-ui-overrides
ENV SQLX_OFFLINE=true

RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

COPY src ./src
RUN touch src/main.rs

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -ms /bin/bash appuser

WORKDIR /app

ARG RELEASE_VERSION=dev
ENV APP_VERSION=$RELEASE_VERSION

COPY --from=builder /app/target/release/GC-Stats-API /usr/local/bin/api

RUN chmod +x /usr/local/bin/api && chown appuser:appuser /usr/local/bin/api

USER appuser

EXPOSE 3000

CMD ["/usr/local/bin/api"]
