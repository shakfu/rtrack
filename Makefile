.PHONY: build run test test-unit test-integration fmt clippy lint clean

build:
	cargo build

run:
	cargo run

test:
	cargo test

test-unit:
	cargo test --lib

test-integration:
	cargo test --test integration

fmt:
	cargo fmt

clippy:
	cargo clippy --all-targets -- -D warnings

lint: fmt clippy

clean:
	cargo clean
