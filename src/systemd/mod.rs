// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

pub mod cpu;
pub mod cpuset;
pub mod dbus;
pub use dbus::{
    SystemdClient, BLOCK_IO_ACCOUNTING, CPU_ACCOUNTING, IO_ACCOUNTING, MEMORY_ACCOUNTING,
    TASKS_ACCOUNTING,
};
pub mod error;
pub mod memory;
pub mod pids;
pub mod utils;

use zbus::zvariant::Value as ZbusValue;

use crate::fs::hierarchies;

pub const DEFAULT_SLICE: &str = "system.slice";

pub const SLICE_SUFFIX: &str = ".slice";
pub const SCOPE_SUFFIX: &str = ".scope";

pub const CPU_SYSTEMD_VERSION: usize = 242;
pub const CPUSET_SYSTEMD_VERSION: usize = 244;

pub type Property<'a> = (&'a str, ZbusValue<'a>);

pub fn cgroup_properties() -> Vec<Property<'static>> {
    let v2 = hierarchies::is_cgroup2_unified_mode();

    let mut props = vec![
        (CPU_ACCOUNTING, ZbusValue::Bool(true)),
        // MemoryAccount is for cgroupsv2 as documented in dbus.
        // However, "github.com/opencontainer/runc" uses it for all.
        // Shall we follow the same way?
        (MEMORY_ACCOUNTING, ZbusValue::Bool(true)),
        (TASKS_ACCOUNTING, ZbusValue::Bool(true)),
    ];

    if v2 {
        props.push((IO_ACCOUNTING, ZbusValue::Bool(true)));
    } else {
        props.push((BLOCK_IO_ACCOUNTING, ZbusValue::Bool(true)));
    }

    props
}
