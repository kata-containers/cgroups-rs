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
test: test-systemd
	cargo test -- --color always --nocapture \
	  --skip systemd::dbus::client::tests

.PHONY: test-systemd
# Tests that manipulate cgroups should run in sequence, so that
# `--test-threads=1` is used.
test-systemd:
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

