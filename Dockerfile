# Builder
FROM rust:1.96-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates/whatbot/Cargo.toml crates/whatbot/Cargo.toml
COPY crates/whatbot-core/Cargo.toml crates/whatbot-core/Cargo.toml
COPY crates/whatbot-storage/Cargo.toml crates/whatbot-storage/Cargo.toml
COPY crates/whatbot-commands/Cargo.toml crates/whatbot-commands/Cargo.toml
COPY crates/whatbot-io-console/Cargo.toml crates/whatbot-io-console/Cargo.toml
COPY crates/whatbot-io-discord/Cargo.toml crates/whatbot-io-discord/Cargo.toml
COPY crates/whatbot-test-support/Cargo.toml crates/whatbot-test-support/Cargo.toml

# Stolen from another Rust dockerfile, creates stubs to allow dependency caching before we build anything
RUN for crate in whatbot-core whatbot-storage whatbot-commands whatbot-io-console whatbot-io-discord whatbot-test-support; do \
        mkdir -p crates/$crate/src && echo "pub fn _stub() {}" > crates/$crate/src/lib.rs; \
    done && \
    mkdir -p crates/whatbot/src && echo "fn main() {}" > crates/whatbot/src/main.rs

RUN cargo build --release --bin whatbot 2>/dev/null || true

COPY crates/ crates/
COPY migrations migrations/
RUN touch crates/*/src/*.rs && cargo build --release --bin whatbot


# Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --no-create-home --shell /usr/sbin/nologin whatbot

WORKDIR /app

COPY --from=builder /build/target/release/whatbot /app/whatbot

RUN mkdir /app/conf

USER whatbot

# Mount your config file at /app/conf/whatbot.toml, or set WHATBOT_CONFIG
# to point at any path inside the container.
ENV WHATBOT_CONFIG=/app/conf/whatbot.toml

CMD ["/app/whatbot"]
