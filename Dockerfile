FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY cmd/shard/Cargo.toml cmd/shard/
COPY core/Cargo.toml core/
COPY net/Cargo.toml net/
COPY crypto/Cargo.toml crypto/
COPY storage/Cargo.toml storage/
RUN mkdir -p cmd/shard/src core/src net/src crypto/src storage/src && \
    echo "fn main() {}" > cmd/shard/src/main.rs && \
    touch core/src/lib.rs net/src/lib.rs crypto/src/lib.rs storage/src/lib.rs && \
    cargo build --release 2>/dev/null || true
COPY cmd/ cmd/
COPY core/ core/
COPY net/ net/
COPY crypto/ crypto/
COPY storage/ storage/
RUN cargo build --release --bin shard

FROM alpine:3.20
RUN apk add --no-cache ca-certificates
COPY --from=builder /build/target/release/shard /usr/local/bin/shard
ENTRYPOINT ["shard"]
