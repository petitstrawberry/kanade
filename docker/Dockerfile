FROM rust:1.94.1-trixie AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/kanade
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/src/kanade/target \
    cargo build --release -p kanade -p kanade-node && \
    mv target/release/kanade /tmp/kanade && \
    mv target/release/kanade-node /tmp/kanade-node

FROM debian:trixie-20260406-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/kanade /usr/local/bin/kanade
COPY --from=builder /tmp/kanade-node /usr/local/bin/kanade-node

EXPOSE 8080 8081 8090
ENTRYPOINT ["kanade"]
