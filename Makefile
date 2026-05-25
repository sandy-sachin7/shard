.PHONY: all build check test test-all bench lint fmt clippy audit dev-ci doc clean install uninstall

all: build

build:
	cargo build

check:
	cargo check

test:
	cargo test

test-all:
	cargo test --all-targets

bench:
	cargo bench

lint: fmt clippy audit

fmt:
	cargo fmt --check

clippy:
	cargo clippy --all-targets -- -D warnings

audit:
	cargo audit

dev-ci: check fmt clippy test bench

doc:
	cargo doc --no-deps

clean:
	cargo clean

install:
	cargo install --path cmd/shard-cli

uninstall:
	cargo uninstall shard

example:
	cargo run --example shard-basics -p shard-core
