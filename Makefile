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

# Tests that manipulate cgroups should run in sequence, so that
# `--test-threads=1` is used.
test: test-systemd test-fs-manager test-systemd-manager
	cargo test --all-features -- --color always \
	  --nocapture \
	  --skip systemd::dbus::client::tests \
	  --skip manager::fs::tests \
	  --skip manager::systemd::tests

.PHONY: test-systemd
# Tests that manipulate cgroups should run in sequence, so that
# `--test-threads=1` is used.
test-systemd:
	cargo test --package cgroups-rs --lib \
	    -- systemd::dbus::client::tests \
	    --color always --nocapture \
		--test-threads=1

.PHONY: test-fs-manager
# See test-systemd
test-fs-manager:
	cargo test --all-features --package cgroups-rs \
		--lib -- manager::fs::tests \
		--color always --nocapture --test-threads=1

.PHONY: test-systemd-manager
# See test-systemd
test-systemd-manager:
	cargo test --all-features --package cgroups-rs \
	  --lib -- manager::systemd::tests \
	  --color always --nocapture --test-threads=1

.PHONY: check
check: fmt clippy


.PHONY: fmt
fmt:
	cargo fmt --all -- --check

.PHONY: clippy
clippy:
	cargo clippy --all-targets --all-features -- -D warnings

