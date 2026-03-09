.PHONY: build run test test-unit test-integration fmt clippy lint clean

build:
	cargo build --workspace

run:
	cargo run -p rtrack-tui

test:
	cargo test --workspace

test-unit:
	cargo test --workspace --lib

test-integration:
	cargo test -p rtrack-tui --test integration

fmt:
	cargo fmt --all

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

lint: fmt clippy

clean:
	cargo clean
