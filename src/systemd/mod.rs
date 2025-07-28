// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

pub mod cpu;
pub mod cpuset;
pub mod dbus;
pub use dbus::SystemdClient;
mod consts;
pub use consts::*;
pub mod error;
pub mod memory;
pub mod pids;
pub mod props;
pub use props::Property;
pub mod utils;

pub const DEFAULT_SLICE: &str = "system.slice";

pub const SLICE_SUFFIX: &str = ".slice";
pub const SCOPE_SUFFIX: &str = ".scope";

pub const CPU_SYSTEMD_VERSION: usize = 242;
pub const CPUSET_SYSTEMD_VERSION: usize = 244;
