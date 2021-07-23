all: debug fmt test

#
# Build
#

.PHONY: debug
debug:
	RUSTFLAGS="--deny warnings" cargo build --all-features

.PHONY: release
release:
	cargo build --release --all-features

.PHONY: build
build: debug

#
# Tests and linters
#

.PHONY: test
test:
	cargo test --all-features -- --color always --nocapture

.PHONY: check
check: fmt clippy


.PHONY: fmt
fmt:
	cargo fmt --all -- --check

.PHONY: clippy
clippy:
	cargo clippy --all-targets --all-features -- -D warnings

