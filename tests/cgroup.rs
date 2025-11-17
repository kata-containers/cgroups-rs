// Copyright (c) 2018 Levente Kurusa
// Copyright (c) 2020 And Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

//! Simple unit tests about the control groups system.
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

use cgroups_rs::fs::cgroup::{
    CGROUP_MODE_DOMAIN, CGROUP_MODE_DOMAIN_INVALID, CGROUP_MODE_DOMAIN_THREADED,
    CGROUP_MODE_THREADED,
};
use cgroups_rs::fs::memory::MemController;
use cgroups_rs::fs::Controller;
use cgroups_rs::fs::{Cgroup, Subsystem};
use cgroups_rs::CgroupPid;

#[test]
fn test_procs_iterator_cgroup() {
    let h = cgroups_rs::fs::hierarchies::auto();
    let pid = libc::pid_t::from(nix::unistd::getpid()) as u64;
    let cg = Cgroup::new(h, String::from("test_procs_iterator_cgroup")).unwrap();
    {
        // Add a task to the control group.
        cg.add_task_by_tgid(CgroupPid::from(pid)).unwrap();

        let mut procs = cg.procs().into_iter();
        // Verify that the task is indeed in the xcontrol group
        assert_eq!(procs.next(), Some(CgroupPid::from(pid)));
        assert_eq!(procs.next(), None);

        // Now, try removing it.
        cg.remove_task_by_tgid(CgroupPid::from(pid)).unwrap();
        procs = cg.procs().into_iter();

        // Verify that it was indeed removed.
        assert_eq!(procs.next(), None);
    }
    cg.delete().unwrap();
}

#[test]
fn test_tasks_iterator_cgroup_v1() {
    if cgroups_rs::fs::hierarchies::is_cgroup2_unified_mode() {
        return;
    }
    let h = cgroups_rs::fs::hierarchies::auto();
    let pid = libc::pid_t::from(nix::unistd::getpid()) as u64;
    let cg = Cgroup::new(h, String::from("test_tasks_iterator_cgroup_v1")).unwrap();
    {
        // Add a task to the control group.
        cg.add_task(CgroupPid::from(pid)).unwrap();

        let mut tasks = cg.tasks().into_iter();
        // Verify that the task is indeed in the xcontrol group
        assert_eq!(tasks.next(), Some(CgroupPid::from(pid)));
        assert_eq!(tasks.next(), None);

        // Now, try removing it.
        cg.remove_task(CgroupPid::from(pid)).unwrap();
        tasks = cg.tasks().into_iter();

        // Verify that it was indeed removed.
        assert_eq!(tasks.next(), None);
    }
    cg.delete().unwrap();
}

#[test]
fn test_tasks_iterator_cgroup_threaded_mode() {
    if !cgroups_rs::fs::hierarchies::is_cgroup2_unified_mode() {
        return;
    }
    let pid = libc::pid_t::from(nix::unistd::getpid()) as u64;
    let cg = Cgroup::new(
        cgroups_rs::fs::hierarchies::auto(),
        String::from("test_tasks_iterator_cgroup_threaded_mode"),
    )
    .unwrap();
    let cg_threaded_sub1 = Cgroup::new_with_specified_controllers(
        cgroups_rs::fs::hierarchies::auto(),
        String::from("test_tasks_iterator_cgroup_threaded_mode/threaded_sub1"),
        Some(vec![String::from("cpuset"), String::from("cpu")]),
    )
    .unwrap();
    let cg_threaded_sub2 = Cgroup::new_with_specified_controllers(
        cgroups_rs::fs::hierarchies::auto(),
        String::from("test_tasks_iterator_cgroup_threaded_mode/threaded_sub2"),
        Some(vec![String::from("cpuset"), String::from("cpu")]),
    )
    .unwrap();
    {
        // Verify that cgroup type of the control group is domain mode.
        assert_eq!(cg.get_cgroup_type().unwrap(), CGROUP_MODE_DOMAIN);

        // Set cgroup type of the sub-control group is thread mode.
        cg_threaded_sub1
            .set_cgroup_type(CGROUP_MODE_THREADED)
            .unwrap();
        // Verify that cgroup type of the sub-control group is thread mode.
        assert_eq!(
            cg_threaded_sub1.get_cgroup_type().unwrap(),
            CGROUP_MODE_THREADED
        );
        // Verify that the cgroup type of the sub-control group that does
        // not set the cgroup type is domain invalid mode.
        assert_eq!(
            cg_threaded_sub2.get_cgroup_type().unwrap(),
            CGROUP_MODE_DOMAIN_INVALID
        );
        // Verify whether the cgroup type of the parent control group of
        // the control group whose cgroup type is set to thread mode is
        // domain thread mode.
        assert_eq!(cg.get_cgroup_type().unwrap(), CGROUP_MODE_DOMAIN_THREADED);

        // Set cgroup type of the sub-control group is thread mode.
        cg_threaded_sub2
            .set_cgroup_type(CGROUP_MODE_THREADED)
            .unwrap();
        // Verify that cgroup type of the sub-control group is thread mode.
        assert_eq!(
            cg_threaded_sub2.get_cgroup_type().unwrap(),
            CGROUP_MODE_THREADED
        );

        // Add a proc to the control group.
        cg.add_task_by_tgid(CgroupPid::from(pid)).unwrap();

        let mut procs = cg.procs().into_iter();
        // Verify that the task is indeed in the x control group
        assert_eq!(procs.next(), Some(CgroupPid::from(pid)));
        assert_eq!(procs.next(), None);

        // Add a task to the sub control group.
        cg_threaded_sub1.add_task(CgroupPid::from(pid)).unwrap();

        let mut tasks = cg_threaded_sub1.tasks().into_iter();
        // Verify that the task is indeed in the xcontrol group
        assert_eq!(tasks.next(), Some(CgroupPid::from(pid)));
        assert_eq!(tasks.next(), None);

        // Now, try move it to parent.
        cg_threaded_sub1
            .move_task_to_parent(CgroupPid::from(pid))
            .unwrap();
        tasks = cg_threaded_sub1.tasks().into_iter();

        // Verify that it was indeed removed.
        assert_eq!(tasks.next(), None);

        // Now, try removing it.
        cg.remove_task_by_tgid(CgroupPid::from(pid)).unwrap();
        procs = cg.procs().into_iter();

        // Verify that it was indeed removed.
        assert_eq!(procs.next(), None);
    }
    cg_threaded_sub1.delete().unwrap();
    cg_threaded_sub2.delete().unwrap();
    cg.delete().unwrap();
}

#[test]
fn test_kill_cgroup() {
    if !cgroups_rs::fs::hierarchies::is_cgroup2_unified_mode() {
        return;
    }
    let h = cgroups_rs::fs::hierarchies::auto();
    let cg = Cgroup::new(h, String::from("test_kill_cgroup")).unwrap();
    {
        // Spawn a proc, don't want to getpid(2) here.
        let mut child = Command::new("sleep").arg("infinity").spawn().unwrap();
        cg.add_task_by_tgid(CgroupPid::from(child.id() as u64))
            .unwrap();

        let cg_procs = cg.procs();
        assert_eq!(cg_procs.len(), 1_usize);

        // Now kill and wait on the proc.
        cg.kill().unwrap();

        let mut tries = 0;
        let status: Option<std::process::ExitStatus> = loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    break Some(status);
                }
                Ok(None) => {
                    if tries > 3 {
                        break None;
                    }
                    sleep(Duration::from_millis(100));
                    tries += 1;
                }
                Err(e) => {
                    child.kill().unwrap();
                    panic!("error attempting to wait: {}", e);
                }
            }
        };
        assert!(status.is_some());
    }
    cg.delete().unwrap();
}

#[test]
fn test_cgroup_with_relative_paths() {
    if cgroups_rs::fs::hierarchies::is_cgroup2_unified_mode() {
        return;
    }
    let h = cgroups_rs::fs::hierarchies::auto();
    let cgroup_root = h.root();
    let cgroup_name = "test_cgroup_with_relative_paths";

    let cg = Cgroup::load(h, String::from(cgroup_name));
    {
        let subsystems = cg.subsystems();
        subsystems.iter().for_each(|sub| match sub {
            Subsystem::Pid(c) => {
                let cgroup_path = c.path().to_str().unwrap();
                let relative_path = "/pids/";
                // cgroup_path = cgroup_root + relative_path + cgroup_name
                assert_eq!(
                    cgroup_path,
                    format!(
                        "{}{}{}",
                        cgroup_root.to_str().unwrap(),
                        relative_path,
                        cgroup_name
                    )
                );
            }
            Subsystem::Mem(c) => {
                let cgroup_path = c.path().to_str().unwrap();
                // cgroup_path = cgroup_root + relative_path + cgroup_name
                assert_eq!(
                    cgroup_path,
                    format!("{}/memory/{}", cgroup_root.to_str().unwrap(), cgroup_name)
                );
            }
            _ => {}
        });
    }
    cg.delete().unwrap();
}

#[test]
fn test_cgroup_v2() {
    if !cgroups_rs::fs::hierarchies::is_cgroup2_unified_mode() {
        return;
    }
    let h = cgroups_rs::fs::hierarchies::auto();
    let cg = Cgroup::new(h, String::from("test_v2")).unwrap();

    let mem_controller: &MemController = cg.controller_of().unwrap();
    let (mem, swp, rev) = (4 * 1024 * 1000, 2 * 1024 * 1000, 1024 * 1000);

    mem_controller.set_limit(mem).unwrap();
    mem_controller.set_memswap_limit(swp).unwrap();
    mem_controller.set_soft_limit(rev).unwrap();

    let memory_stat = mem_controller.memory_stat();
    println!("memory_stat {:?}", memory_stat);
    assert_eq!(mem, memory_stat.limit_in_bytes);
    assert_eq!(rev, memory_stat.soft_limit_in_bytes);

    let memswap = mem_controller.memswap();
    println!("memswap {:?}", memswap);
    assert_eq!(swp, memswap.limit_in_bytes);

    cg.delete().unwrap();
}

#[test]
fn test_delete_on_drop_disabled() {
    let cgroup_name = "test_delete_on_drop_disabled";

    {
        let h = cgroups_rs::fs::hierarchies::auto();
        let cg = Cgroup::new(h, String::from(cgroup_name)).unwrap();
        assert!(cg.exists());
    }

    // Cgroup should still exist after drop since delete_on_drop wasn't set
    let h = cgroups_rs::fs::hierarchies::auto();
    let cg = Cgroup::load(h, String::from(cgroup_name));
    assert!(cg.exists());

    cg.delete().unwrap();
}

#[test]
fn test_delete_on_drop_enabled() {
    let cgroup_name = "test_delete_on_drop_enabled";

    {
        let h = cgroups_rs::fs::hierarchies::auto();
        let mut cg = Cgroup::new(h, String::from(cgroup_name)).unwrap();
        cg.delete_on_drop(true);
        assert!(cg.exists());
    }

    // Cgroup should not exist after drop since delete_on_drop was set
    let h = cgroups_rs::fs::hierarchies::auto();
    let cg = Cgroup::load(h, String::from(cgroup_name));
    assert!(!cg.exists());
}
