// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

/// Who enum: all
pub const WHO_ENUM_ALL: &str = "all";

/// Unit mode: replace
pub const UNIT_MODE_REPLACE: &str = "replace";

/// No such unit error
pub const NO_SUCH_UNIT: &str = "org.freedesktop.systemd1.NoSuchUnit";

/// Default description for transient units.
pub const DEFAULT_DESCRIPTION: &str = "cgroups-rs transient unit";

/// Turn on CPU usage accounting for this unit.
pub const CPU_ACCOUNTING: &str = "CPUAccounting";
/// This setting controls the memory controller in the unified hierarchy.
/// Added in version 208.
pub const MEMORY_ACCOUNTING: &str = "MemoryAccounting";
/// This setting controls the pids controller in the unified hierarchy.
pub const TASKS_ACCOUNTING: &str = "TasksAccounting";
/// This setting controls the io controller in the unified hierarchy.
/// Added in version 230.
pub const IO_ACCOUNTING: &str = "IOAccounting";
/// This setting controls the block IO controller in the legacy hierarchy.
/// Deprecated in version 252.
pub const BLOCK_IO_ACCOUNTING: &str = "BlockIOAccounting";
/// Description of the unit.
pub const DESCRIPTION: &str = "Description";
/// PIDs
pub const PIDS: &str = "PIDs";
/// Default dependencies for this unit.
pub const DEFAULT_DEPENDENCIES: &str = "DefaultDependencies";
/// Wants, expressing a weak dependency on other units.
pub const WANTS: &str = "Wants";
/// Slice, used to assign a unit to a specific slice.
pub const SLICE: &str = "Slice";
/// Turns on delegation of further resource control partitioning to
/// processes of the unit.
pub const DELEGATE: &str = "Delegate";
/// Timeout for stopping the unit in microseconds.
pub const TIMEOUT_STOP_USEC: &str = "TimeoutStopUSec";

/// CPU shares in the legacy hierarchy.
pub const CPU_SHARES: &str = "CPUShares";
/// CPU shares in the unified hierarchy.
pub const CPU_WEIGHT: &str = "CPUWeight";
/// CPU quota period us.
pub const CPU_QUOTA_PERIOD_US: &str = "CPUQuotaPeriodUSec";
/// CPU quota us
pub const CPU_QUOTA_PER_SEC_US: &str = "CPUQuotaPerSecUSec";
/// Allowed CPUs
pub const ALLOWED_CPUS: &str = "AllowedCPUs";
/// Allowed memory nodes
pub const ALLOWED_MEMORY_NODES: &str = "AllowedMemoryNodes";
/// Memory limit in the legacy hierarchy.
pub const MEMORY_LIMIT: &str = "MemoryLimit";
/// Memory limit in the unified hierarchy.
pub const MEMORY_MAX: &str = "MemoryMax";
/// Memory low
pub const MEMORY_LOW: &str = "MemoryLow";
/// Memory swap max
pub const MEMORY_SWAP_MAX: &str = "MemorySwapMax";
/// Tasks max
pub const TASKS_MAX: &str = "TasksMax";
