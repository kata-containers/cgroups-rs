# Reproducible cgroup QEMU test environments

These scripts boot the same pinned Ubuntu 22.04 guest in explicit cgroup v1
and cgroup v2 modes. Both the cgroupfs and systemd backends are therefore
tested against real kernel interfaces without inheriting the GitHub runner's
cgroup hierarchy, systemd version, mount layout, or delegation policy.

The workflow runs the guests with KVM. GitHub-hosted Linux runners provide
`/dev/kvm`, but only to root, so the workflow first opens it to the runner
user with a udev rule. The scripts default to QEMU TCG software emulation, so
they also work on hosts without KVM.

Rust test executables are cross-compiled as static musl binaries on the GitHub
runner, which keeps local runs with TCG practical. Only the completed
executables run inside the guest. `run-tests-in-cgroup-vm.sh` transfers them together with
`run-test-binaries-in-guest.sh`, which contains the guest-side test ordering
and failure cleanup.

The scripts are named for their responsibilities:

| Script | Responsibility |
| --- | --- |
| `build-static-test-binaries.sh` | Cross-compile portable Rust test harnesses |
| `boot-cgroup-test-vm.sh` | Configure, boot, and verify a v1 or v2 QEMU guest |
| `run-tests-in-cgroup-vm.sh` | Transfer tests and coordinate execution over SSH |
| `run-test-binaries-in-guest.sh` | Execute test harnesses inside the guest |
| `stop-cgroup-test-vm.sh` | Stop the disposable QEMU guest |

The test matrix runs entirely in QEMU:

| Guest mode | Filesystem backend | systemd backend |
| --- | --- | --- |
| Ubuntu 22.04, cgroup v1 | cgroup v1 | cgroup v1 |
| Ubuntu 22.04, cgroup v2 | cgroup v2 | cgroup v2 |

Both modes are selected with an explicit
`systemd.unified_cgroup_hierarchy=<0|1>` kernel argument. Each guest must pass
an exact hierarchy check before tests start. This is important because several
existing tests return early when their expected cgroup version is unavailable.
