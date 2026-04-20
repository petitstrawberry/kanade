FROM rust:1.94.1-trixie AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install --locked cargo-about

WORKDIR /usr/src/kanade
COPY . .

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    CARGO_INCREMENTAL=0 cargo build --release -p kanade -p kanade-node && \
    cp target/release/kanade /tmp/kanade && \
    cp target/release/kanade-node /tmp/kanade-node && \
    cargo about generate about.hbs > /tmp/THIRD_PARTY_LICENSES.html

FROM debian:trixie-20260406-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/kanade /usr/local/bin/kanade
COPY --from=builder /tmp/kanade-node /usr/local/bin/kanade-node
COPY --from=builder /usr/src/kanade/LICENSE /usr/share/doc/kanade/LICENSE
COPY --from=builder /tmp/THIRD_PARTY_LICENSES.html /usr/share/doc/kanade/THIRD_PARTY_LICENSES.html

EXPOSE 8080 8090
ENTRYPOINT ["kanade"]
