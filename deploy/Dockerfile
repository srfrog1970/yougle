# Build context must be the repo root, not this directory — this builds
# the whole Cargo workspace pm-node depends on. From the repo root:
#   docker build -f deploy/Dockerfile -t pm-node .
# docker-compose.yml in this directory already points at the right context.

FROM rust:1-slim-bookworm AS builder

# pm-store's `bundled-sqlcipher-vendored-openssl` feature (see
# crates/pm-store/Cargo.toml's own comment on why: no pre-built OpenSSL for
# the target needed) builds OpenSSL from source, which needs a C toolchain
# and perl for its Configure script.
RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential pkg-config perl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release -p pm-node

FROM debian:bookworm-slim

# CA certificates: iroh's relay/discovery infrastructure talks HTTPS.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/pm-node /usr/local/bin/pm-node

# Configuration (PM_NODE_MAILBOX_KEY, PM_NODE_TRANSPORT_KEY,
# PM_NODE_DATA_DIR) comes entirely from the environment — see
# .env.example and ../README.md. /data is the documented default mount
# point for PM_NODE_DATA_DIR in docker-compose.yml; declaring it here is
# just a hint for `docker volume`/`docker inspect`, not itself required
# for persistence to work.
VOLUME ["/data"]

ENTRYPOINT ["/usr/local/bin/pm-node"]
