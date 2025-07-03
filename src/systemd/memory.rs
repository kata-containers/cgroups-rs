// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::systemd::dbus;
use crate::systemd::error::{Error, Result};

/// Returns the property for memory limit.
pub fn limit(limit: i64, v2: bool) -> Result<(&'static str, u64)> {
    let id = if v2 {
        dbus::MEMORY_MAX
    } else {
        dbus::MEMORY_LIMIT
    };

    Ok((id, limit as u64))
}

/// Returns the property for memory limit.
pub fn low(low: i64, v2: bool) -> Result<(&'static str, u64)> {
    if !v2 {
        return Err(Error::CgroupsV1NotSupported);
    }

    Ok((dbus::MEMORY_LOW, low as u64))
}

/// Returns the property for memory swap.
pub fn swap(swap: i64, v2: bool) -> Result<(&'static str, u64)> {
    if !v2 {
        return Err(Error::CgroupsV1NotSupported);
    }

    Ok((dbus::MEMORY_SWAP_MAX, swap as u64))
}
