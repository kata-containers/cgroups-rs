all: debug fmt test

#
# Build
#

.PHONY: debug
debug:
	RUSTFLAGS="--deny warnings" cargo build

.PHONY: release
release:
	cargo build --release

.PHONY: build
build: debug

#
# Tests and linters
#

.PHONY: test
test:
# tests for systemd client should run in sequence, see [1].
#
# 1: https://github.com/kata-containers/cgroups-rs/pull/148
	cargo test -- --color always --nocapture \
	  --skip systemd::dbus::client::tests
	
	cargo test --package cgroups-rs --lib \
	    -- systemd::dbus::client::tests \
	    --color always --nocapture \
		--test-threads=1

.PHONY: check
check: fmt clippy


.PHONY: fmt
fmt:
	cargo fmt --all -- --check

.PHONY: clippy
clippy:
	cargo clippy --all-targets --all-features -- -D warnings

