FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY cmd/shard-cli/Cargo.toml cmd/shard-cli/
COPY core/Cargo.toml core/
COPY net/Cargo.toml net/
COPY crypto/Cargo.toml crypto/
COPY storage/Cargo.toml storage/
COPY python/shard-py/Cargo.toml python/shard-py/
RUN mkdir -p cmd/shard-cli/src core/src net/src crypto/src storage/src python/shard-py/src tests/src && \
    echo "fn main() {}" > cmd/shard-cli/src/main.rs && \
    touch core/src/lib.rs net/src/lib.rs crypto/src/lib.rs storage/src/lib.rs python/shard-py/src/lib.rs tests/src/lib.rs && \
    cargo build --release 2>/dev/null || true
COPY cmd/ cmd/
COPY core/ core/
COPY net/ net/
COPY crypto/ crypto/
COPY storage/ storage/
COPY python/ python/
COPY tests/ tests/
COPY .cargo/ .cargo/
RUN cargo build --release --bin shard

FROM alpine:3.20
RUN apk add --no-cache ca-certificates
COPY --from=builder /build/target/release/shard /usr/local/bin/shard
ENTRYPOINT ["shard"]
