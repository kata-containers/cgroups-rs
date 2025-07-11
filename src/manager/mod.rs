// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

mod error;
use std::collections::HashMap;

pub use error::{Error, Result};
mod fs;
pub use fs::FsManager;
mod conv;

use oci_spec::runtime::LinuxResources;

use crate::{CgroupPid, CgroupStats, FreezerState};

/// Manage cgroups designed for OCI containers.
pub trait Manager: Send + Sync {
    /// Add a process specified by its tgid.
    fn add_proc(&mut self, tgid: CgroupPid) -> Result<()>;

    /// Add a thread specified by its pid.
    fn add_thread(&mut self, pid: CgroupPid) -> Result<()>;

    /// Get the list of pids joint to the cgroups.
    fn pids(&self) -> Result<Vec<CgroupPid>>;

    /// Set the freezer cgroup to the specified state.
    fn freeze(&self, state: FreezerState) -> Result<()>;

    /// Remove the cgroups.
    fn destroy(&mut self) -> Result<()>;

    /// Set the resources to the cgroups.
    fn set(&mut self, resources: &LinuxResources) -> Result<()>;

    /// Get the cgroup path.
    ///
    /// # Arguments
    ///
    /// - `subsystem`: cgroup subsystem, for cgroup v1 the value should not
    ///   be empty, while for cgroup v2 the only valid value is `None`.
    fn cgroup_path(&self, subsystem: Option<&str>) -> Result<String>;

    /// Enable CPUs, topdown from root in cgroup hierarchy, this would be
    /// useful for CPU hotplug in the guest.
    ///
    /// The caller should update cgroup resources manually, in particular
    /// cpuset, after this, in order to use the new CPUs (or avoid using
    /// offline CPUs).
    ///
    /// # Arguments
    ///
    /// - `cpus`: online CPUs in the same format with `cat
    ///   /sys/devices/system/cpu/online`, e.g. "0-3,6-7".
    fn enable_cpus_topdown(&self, cpus: &str) -> Result<()>;

    /// Get cgroup stats.
    fn stats(&self) -> CgroupStats;

    /// Get the mappings of subsystems to their relative path. The full
    /// path would be something like "{mountpoint}/{relative_path}". The
    /// mappings of mountpoints see "mounts()".
    fn paths(&self) -> &HashMap<String, String>;

    /// Get the mappings of subsystems to their mountpoints. The full
    /// path would be something like "{mountpoint}/{relative_path}". The
    /// mappings of relative paths see "paths()".
    fn mounts(&self) -> &HashMap<String, String>;

    /// Indicate whether the cgroup manager is using systemd.
    fn systemd(&self) -> bool;

    /// Indicate whether the cgroup manager is using cgroup v2.
    fn v2(&self) -> bool;
}
