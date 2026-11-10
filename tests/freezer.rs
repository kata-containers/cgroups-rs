// Copyright (c) 2018 Levente Kurusa
// Copyright (c) 2020 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

//! Integration tests for the freezer controller on both cgroup versions.
//!
//! These tests move a forked child process into the cgroup and freeze it,
//! keeping the test process itself outside so it can thaw the child again.

use std::process::{Child, Command};
use std::thread::sleep;
use std::time::Duration;

use cgroups_rs::fs::error::ErrorKind;
use cgroups_rs::fs::freezer::FreezerController;
use cgroups_rs::fs::Cgroup;
use cgroups_rs::{CgroupPid, FreezerState};

/// Polls the freezer state until it reaches the expected value, tolerating
/// transient states such as `Freezing`.
fn wait_for_state(freezer: &FreezerController, expected: FreezerState) {
    for _ in 0..100 {
        if freezer.state().unwrap() == expected {
            return;
        }
        sleep(Duration::from_millis(10));
    }
    assert_eq!(freezer.state().unwrap(), expected);
}

/// Moves `task` into the cgroup, freezes it, verifies the state, thaws it
/// and moves the task back out.
///
/// On v2 the task is moved through `cgroup.procs` (`add_task_by_tgid`),
/// because `add_task` writes `cgroup.threads`, which is only writable in
/// threaded mode. On v1 the issue's original `add_task` flow is used.
fn freeze_and_thaw(cg: &Cgroup, task: u64) {
    let freezer: &FreezerController = cg.controller_of().expect("freezer controller not found");
    let task = CgroupPid::from(task);

    if cg.v2() {
        cg.add_task_by_tgid(task).unwrap();
    } else {
        cg.add_task(task).unwrap();
    }
    freezer.freeze().unwrap();
    wait_for_state(freezer, FreezerState::Frozen);
    freezer.thaw().unwrap();
    wait_for_state(freezer, FreezerState::Thawed);
    if cg.v2() {
        cg.remove_task_by_tgid(task).unwrap();
    } else {
        cg.remove_task(task).unwrap();
    }
}

struct ChildGuard<'a> {
    child: Child,
    cgroup: &'a Cgroup,
}

impl<'a> Drop for ChildGuard<'a> {
    fn drop(&mut self) {
        // A task frozen by the v1 (or v2) freezer ignores SIGKILL, so a
        // panic that leaves the child frozen would make `wait()` block
        // until the CI timeout. Thaw the cgroup best-effort first; on an
        // already-thawed cgroup `thaw()` is an idempotent no-op write.
        if let Some(freezer) = self.cgroup.controller_of::<FreezerController>() {
            let _ = freezer.thaw();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Forks a child that just sleeps, runs `f` in the parent, then kills and
/// reaps the child.
fn with_sleeping_child(cg: &Cgroup, f: impl FnOnce(u64)) {
    let child = Command::new("sleep")
        .arg("60")
        .spawn()
        .expect("Failed to spawn sleep process");

    let guard = ChildGuard { child, cgroup: cg };
    f(guard.child.id() as u64);
}

#[test]
fn test_freezer_v2_specified_controllers() {
    if !cgroups_rs::fs::hierarchies::is_cgroup2_unified_mode() {
        return;
    }

    // Regression test for issue #124: creating a v2 cgroup with only the
    // freezer controller specified must succeed, even though the kernel
    // does not list freezer in cgroup.controllers.
    let cg = match Cgroup::new_with_specified_controllers(
        cgroups_rs::fs::hierarchies::auto(),
        String::from("test_freezer_v2_specified_controllers"),
        Some(vec![String::from("freezer")]),
    ) {
        Ok(cg) => cg,
        Err(e) if e.kind() == &ErrorKind::SpecifiedControllers => return,
        Err(e) => panic!("failed to create v2 cgroup: {}", e),
    };

    with_sleeping_child(&cg, |pid| {
        freeze_and_thaw(&cg, pid);
    });
    cg.delete().unwrap();
}

#[test]
fn test_freezer_v1_freeze_thaw() {
    if cgroups_rs::fs::hierarchies::is_cgroup2_unified_mode() {
        return;
    }

    let cg = Cgroup::new(
        cgroups_rs::fs::hierarchies::auto(),
        String::from("test_freezer_v1_freeze_thaw"),
    )
    .unwrap();

    // Skip when the freezer subsystem is not mounted on this host.
    if cg.controller_of::<FreezerController>().is_none() {
        cg.delete().unwrap();
        return;
    }

    with_sleeping_child(&cg, |pid| {
        freeze_and_thaw(&cg, pid);
    });
    cg.delete().unwrap();
}
