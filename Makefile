.PHONY: build run gui test test-unit test-integration fmt clippy lint clean \
       publish-dry publish publish-core publish-tui publish-gui

build:
	cargo build --workspace

run:
	cargo run -p rtrack-tui

gui:
	cargo run -p rtrack-gui

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

publish-dry:
	cargo publish -p rtrack-core --dry-run
	cargo publish -p rtrack-tui --dry-run
	cargo publish -p rtrack-gui --dry-run

publish-core:
	cargo publish -p rtrack-core

publish-tui:
	cargo publish -p rtrack-tui

publish-gui:
	cargo publish -p rtrack-gui

publish: publish-core publish-tui publish-gui
