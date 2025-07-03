// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::systemd::error::{Error, Result};
use crate::systemd::{MEMORY_LIMIT, MEMORY_LOW, MEMORY_MAX, MEMORY_SWAP_MAX};

/// Returns the property for memory limit.
pub fn limit(limit: i64, v2: bool) -> Result<(&'static str, u64)> {
    let id = if v2 { MEMORY_MAX } else { MEMORY_LIMIT };

    Ok((id, limit as u64))
}

/// Returns the property for memory limit.
pub fn low(low: i64, v2: bool) -> Result<(&'static str, u64)> {
    if !v2 {
        return Err(Error::CgroupsV1NotSupported);
    }

    Ok((MEMORY_LOW, low as u64))
}

/// Returns the property for memory swap.
pub fn swap(swap: i64, v2: bool) -> Result<(&'static str, u64)> {
    if !v2 {
        return Err(Error::CgroupsV1NotSupported);
    }

    Ok((MEMORY_SWAP_MAX, swap as u64))
}
