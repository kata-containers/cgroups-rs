// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::systemd::error::{Error, Result};
use crate::systemd::props::Value;
use crate::systemd::{Property, MEMORY_LIMIT, MEMORY_LOW, MEMORY_MAX, MEMORY_SWAP_MAX};

/// Returns the property for memory limit.
pub fn limit(limit: i64, v2: bool) -> Result<Property> {
    let id = if v2 { MEMORY_MAX } else { MEMORY_LIMIT };

    Ok((id.to_string(), Value::U64(limit as u64)))
}

/// Returns the property for memory limit.
pub fn low(low: i64, v2: bool) -> Result<Property> {
    if !v2 {
        return Err(Error::CgroupsV1NotSupported);
    }

    Ok((MEMORY_LOW.to_string(), Value::U64(low as u64)))
}

/// Returns the property for memory swap.
pub fn swap(swap: i64, v2: bool) -> Result<Property> {
    if !v2 {
        return Err(Error::CgroupsV1NotSupported);
    }

    Ok((MEMORY_SWAP_MAX.to_string(), Value::U64(swap as u64)))
}
