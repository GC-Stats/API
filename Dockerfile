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

RUN curl -sL https://github.com/tailwindlabs/tailwindcss/releases/download/v4.1.8/tailwindcss-linux-x64 \
    -o /usr/local/bin/tailwindcss && chmod +x /usr/local/bin/tailwindcss

COPY assets ./assets
COPY templates ./templates
COPY src ./src
RUN touch src/main.rs

RUN tailwindcss -i assets/input.css -o assets/tailwind.css --minify

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    libssl3 \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -ms /bin/bash appuser

WORKDIR /app

COPY --from=builder /app/target/release/GC-Stats-API /usr/local/bin/api
COPY --from=builder /app/assets/tailwind.css ./assets/tailwind.css

RUN chmod +x /usr/local/bin/api && chown appuser:appuser /usr/local/bin/api
RUN chown -R appuser:appuser /app/assets

USER appuser

EXPOSE 3000

CMD ["/usr/local/bin/api"]
