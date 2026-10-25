.PHONY: build run gui test test-unit test-integration fmt fmt-check clippy doc lint ci clean fuzz \
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

# Fuzz the file parsers, FUZZ_SECS seconds each. Needs a nightly toolchain
# and cargo-fuzz, so CI does not run it. Seeds: fuzz/seeds and examples/.
FUZZ_SECS ?= 60
fuzz:
	cd fuzz && for t in rtrk midi wav aiff; do \
		mkdir -p corpus/$$t && \
		seeds=seeds/$$t; [ $$t = rtrk ] && seeds=../examples; \
		cargo +nightly fuzz run $$t corpus/$$t $$seeds -- \
			-max_total_time=$(FUZZ_SECS) -rss_limit_mb=2048 -max_len=262144 || exit 1; \
	done

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
