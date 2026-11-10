// Copyright (c) 2026 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

//! Tests for the misc cgroup subsystem.

use std::collections::HashMap;
use std::fs::{self, File};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use cgroups_rs::fs::misc::MiscController;
use cgroups_rs::fs::{Cgroup, Controller, MiscMaxValue, MiscResources, Resources};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cgroups-rs-misc-test-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_misc_controller_read_files() {
    let dir = test_dir();

    fs::write(dir.join("misc.capacity"), "sev 50\nsev_es 100\n").unwrap();
    fs::write(dir.join("misc.current"), "sev 10\nsev_es 20\n").unwrap();
    fs::write(dir.join("misc.peak"), "sev 15\nsev_es 25\n").unwrap();
    fs::write(dir.join("misc.max"), "sev 40\nsev_es max\n").unwrap();
    fs::write(dir.join("misc.events"), "sev.max 2\n").unwrap();
    fs::write(dir.join("misc.events.local"), "sev.max 1\n").unwrap();

    let controller = MiscController::new(dir.clone(), dir.clone(), true);

    assert_eq!(controller.capacity().unwrap(), "sev 50\nsev_es 100");
    assert_eq!(controller.current().unwrap(), "sev 10\nsev_es 20");
    assert_eq!(controller.peak().unwrap(), "sev 15\nsev_es 25");
    assert_eq!(controller.max().unwrap(), "sev 40\nsev_es max");
    assert_eq!(controller.events().unwrap(), "sev.max 2");
    assert_eq!(controller.events_local().unwrap(), "sev.max 1");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_misc_controller_set_max() {
    let dir = test_dir();
    let max_file = dir.join("misc.max");
    File::create(&max_file).unwrap();

    let controller = MiscController::new(dir.clone(), dir.clone(), true);
    controller.set_max("sev", MiscMaxValue::Value(42)).unwrap();

    let content = fs::read_to_string(&max_file).unwrap();
    assert_eq!(content, "sev 42");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_misc_controller_apply() {
    let dir = test_dir();
    let max_file = dir.join("misc.max");
    File::create(&max_file).unwrap();

    let controller = MiscController::new(dir.clone(), dir.clone(), true);

    let mut maximum = HashMap::new();
    maximum.insert("sev".to_string(), MiscMaxValue::Value(123));

    let res = Resources {
        misc: MiscResources { maximum },
        ..Default::default()
    };

    controller.apply(&res).unwrap();

    let content = fs::read_to_string(&max_file).unwrap();
    assert_eq!(content, "sev 123");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_misc_controller_apply_max() {
    let dir = test_dir();
    let max_file = dir.join("misc.max");
    File::create(&max_file).unwrap();

    let controller = MiscController::new(dir.clone(), dir.clone(), true);

    let mut maximum = HashMap::new();
    maximum.insert("sev".to_string(), MiscMaxValue::Max);

    let res = Resources {
        misc: MiscResources { maximum },
        ..Default::default()
    };

    controller.apply(&res).unwrap();

    let content = fs::read_to_string(&max_file).unwrap();
    assert_eq!(content, "sev max");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_misc_controller_apply_empty() {
    let dir = test_dir();
    let max_file = dir.join("misc.max");
    File::create(&max_file).unwrap();

    let controller = MiscController::new(dir.clone(), dir.clone(), true);

    // No misc limits: apply must be a no-op and leave misc.max untouched.
    controller.apply(&Resources::default()).unwrap();

    let content = fs::read_to_string(&max_file).unwrap();
    assert_eq!(content, "");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_misc_controller_read_missing_files() {
    let dir = test_dir();
    // The cgroup directory itself does not exist: all reads must fail.
    let missing = dir.join("no-such-cgroup");
    let controller = MiscController::new(missing, dir.clone(), true);

    assert!(controller.capacity().is_err());
    assert!(controller.current().is_err());
    assert!(controller.peak().is_err());
    assert!(controller.max().is_err());
    assert!(controller.events().is_err());
    assert!(controller.events_local().is_err());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_misc_controller_set_max_missing_dir() {
    let dir = test_dir();
    // The cgroup directory itself does not exist: creating misc.max fails.
    let missing = dir.join("no-such-cgroup");
    let controller = MiscController::new(missing, dir.clone(), true);

    assert!(controller.set_max("sev", MiscMaxValue::Value(1)).is_err());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn test_misc_cgroup_create_and_delete() {
    // Runs on both v1 and v2: on v1 it exercises the legacy misc hierarchy
    // when it is mounted; when misc is not available, `controller_of` returns
    // None and the reads are skipped.
    let h = cgroups_rs::fs::hierarchies::auto();
    let cg = Cgroup::new(h, String::from("test_misc_cgroup_create_and_delete")).unwrap();
    {
        let controller: Option<&MiscController> = cg.controller_of();
        if let Some(c) = controller {
            let _ = c.current();
            let _ = c.max();
        }
    }
    cg.delete().unwrap();
}
