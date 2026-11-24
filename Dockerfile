FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig cmake make perl
WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY cmd/shard-cli/Cargo.toml cmd/shard-cli/
COPY core/Cargo.toml core/
COPY net/Cargo.toml net/
COPY crypto/Cargo.toml crypto/
COPY storage/Cargo.toml storage/
COPY tests/Cargo.toml tests/
COPY .cargo/ .cargo/

COPY cmd/ cmd/
COPY core/ core/
COPY net/ net/
COPY crypto/ crypto/
COPY storage/ storage/
COPY tests/ tests/

RUN cargo build --release --locked --bin shard

FROM alpine:3.20
RUN apk add --no-cache ca-certificates && \
    addgroup -S shard && \
    adduser -S -u 10001 -G shard -h /data shard && \
    mkdir -p /data && \
    chown -R shard:shard /data

COPY --from=builder /build/target/release/shard /usr/local/bin/shard

USER shard
WORKDIR /data
ENTRYPOINT ["shard"]
