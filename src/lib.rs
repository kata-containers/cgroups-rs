// Copyright (c) 2018 Levente Kurusa
// Copyright (c) 2020-2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

pub mod fs;
#[cfg(feature = "oci")]
pub mod manager;
#[cfg(feature = "oci")]
pub use manager::{FsManager, Manager, SystemdManager};
pub mod stats;
pub use stats::CgroupStats;
pub mod systemd;

/// The maximum value for CPU shares in cgroups v1
pub const CPU_SHARES_V1_MAX: u64 = 262144;
/// The maximum value for CPU weight in cgroups v2
pub const CPU_WEIGHT_V2_MAX: u64 = 10000;

/// The current state of the control group
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FreezerState {
    /// The processes in the control group are _not_ frozen.
    Thawed,
    /// The processes in the control group are in the processes of being frozen.
    Freezing,
    /// The processes in the control group are frozen.
    Frozen,
}

/// A structure representing a `pid`. Currently implementations exist for `u64` and
/// `std::process::Child`.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct CgroupPid {
    /// The process identifier
    pub pid: u64,
}

impl From<u64> for CgroupPid {
    fn from(u: u64) -> CgroupPid {
        CgroupPid { pid: u }
    }
}

impl From<&std::process::Child> for CgroupPid {
    fn from(u: &std::process::Child) -> CgroupPid {
        CgroupPid { pid: u.id() as u64 }
    }
}
