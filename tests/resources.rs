// Copyright (c) 2018 Levente Kurusa
// Copyright (c) 2020 And Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

//! Integration test about setting resources using `apply()`
use std::collections::HashMap;

use cgroups_rs::fs::pid::PidController;
use cgroups_rs::fs::{Cgroup, MaxValue, MiscMaxValue, MiscResources, PidResources, Resources};

#[test]
fn pid_resources() {
    let h = cgroups_rs::fs::hierarchies::auto();
    let cg = Cgroup::new(h, String::from("pid_resources")).unwrap();
    {
        let res = Resources {
            pid: PidResources {
                maximum_number_of_processes: Some(MaxValue::Value(512)),
            },
            ..Default::default()
        };
        cg.apply(&res).unwrap();

        // verify
        let pidcontroller: &PidController = cg.controller_of().unwrap();
        let pid_max = pidcontroller.get_pid_max();
        assert!(pid_max.is_ok());
        assert_eq!(pid_max.unwrap(), MaxValue::Value(512));
    }
    cg.delete().unwrap();
}

#[test]
fn misc_resources() {
    let h = cgroups_rs::fs::hierarchies::auto();

    let root_misc = cgroups_rs::fs::misc::MiscController::new(
        std::path::PathBuf::from("/sys/fs/cgroup"),
        std::path::PathBuf::from("/sys/fs/cgroup"),
        true,
    );
    let capacity = root_misc.capacity().unwrap_or_default();
    let first_res = capacity
        .lines()
        .find_map(|line| line.split_whitespace().next());

    let cg = Cgroup::new(h, String::from("misc_resources")).unwrap();
    {
        let mut maximum = HashMap::new();
        if let Some(res_name) = first_res {
            maximum.insert(res_name.to_string(), MiscMaxValue::Max);
        }
        let res = Resources {
            misc: MiscResources { maximum },
            ..Default::default()
        };
        cg.apply(&res).unwrap();
    }
    cg.delete().unwrap();
}
