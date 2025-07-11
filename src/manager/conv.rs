// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::manager::error::{Error, Result};
use crate::{CPU_SHARES_V1_MAX, CPU_WEIGHT_V2_MAX};

// Converts CPU shares, used by cgroup v1, to CPU weight, used by cgroup
// v2.
//
// Cgroup v1 CPU shares has a range of [2^1...2^18], i.e. [2...262144],
// and the default value is 1024.
//
// Cgroup v2 CPU weight has a range of [10^0...10^4], i.e. [1...10000],
// and the default value is 100.
pub(crate) fn cpu_shares_to_cgroup_v2(shares: u64) -> u64 {
    if shares == 0 {
        return 0;
    }
    if shares <= 2 {
        return 1;
    }
    if shares >= CPU_SHARES_V1_MAX {
        return CPU_WEIGHT_V2_MAX;
    }

    (((shares - 2) * 9999) / 262142) + 1
}

// ConvertMemorySwapToCgroupV2Value converts MemorySwap value from OCI spec
// for use by cgroup v2 drivers. A conversion is needed since
// Resources.MemorySwap is defined as memory+swap combined, while in cgroup
// v2 swap is a separate value.
pub(crate) fn memory_swap_to_cgroup_v2(memswap_limit: i64, mem_limit: i64) -> Result<i64> {
    // For compatibility with cgroup1 controller, set swap to unlimited in
    // case the memory is set to unlimited, and swap is not explicitly set,
    // treating the request as "set both memory and swap to unlimited".
    if mem_limit == -1 && memswap_limit == 0 {
        return Ok(-1);
    }

    // -1 is "max", 0 is "unset", so treat as is
    if memswap_limit == -1 || memswap_limit == 0 {
        return Ok(memswap_limit);
    }

    // Unlimited memory, so treat swap as is.
    if mem_limit == -1 {
        return Ok(memswap_limit);
    }

    // Unset or unknown memory, can't calculate swap.
    if mem_limit == 0 {
        return Err(Error::InvalidLinuxResource);
    }

    // Does not make sense to subtract a negative value.
    if mem_limit < 0 {
        return Err(Error::InvalidLinuxResource);
    }

    // Sanity check.
    if memswap_limit < mem_limit {
        return Err(Error::InvalidLinuxResource);
    }

    Ok(memswap_limit - mem_limit)
}
