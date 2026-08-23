.PHONY: build run gui test test-unit test-integration fmt fmt-check clippy doc lint ci clean \
       regen-examples check-examples \
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

# Non-mutating counterpart to `fmt`, suitable for gating.
fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets -- -D warnings

# Broken intra-doc links are how a published crate's documentation rots:
# docs.rs renders them as dead text and nothing else complains.
doc:
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Reformats in place, then lints. Use `ci` if you need a check that fails
# rather than rewrites.
lint: fmt clippy

# Rebuild the generated example songs (writes into examples/).
regen-examples:
	cargo xtask regen-examples

# Verify the generated examples are current, without writing.
check-examples:
	cargo xtask regen-examples --check

# What CI runs.
ci: fmt-check clippy doc test check-examples

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
