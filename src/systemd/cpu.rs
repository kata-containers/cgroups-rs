// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::systemd::error::{Error, Result};
use crate::systemd::props::Value;
use crate::systemd::{
    Property, CPU_QUOTA_PERIOD_US, CPU_QUOTA_PER_SEC_US, CPU_SHARES, CPU_SYSTEMD_VERSION,
    CPU_WEIGHT,
};

/// Returns the property for CPU shares.
///
/// Please note that if the shares is obtained from OCI runtime spec, it
/// MUST be converted, see [1] and `convert_shares_to_v2()`.
///
/// 1: https://github.com/containers/crun/blob/main/crun.1.md#cgroup-v2
pub fn shares(shares: u64, v2: bool) -> Result<Property> {
    let id = if v2 { CPU_WEIGHT } else { CPU_SHARES };

    Ok((id.to_string(), Value::U64(shares)))
}

/// Returns the property for CPU period.
pub fn period(period: u64, systemd_version: usize) -> Result<Property> {
    if systemd_version < CPU_SYSTEMD_VERSION {
        return Err(Error::ObsoleteSystemd);
    }

    Ok((CPU_QUOTA_PERIOD_US.to_string(), Value::U64(period)))
}

/// Return the property for CPU quota.
pub fn quota(quota: u64) -> Result<Property> {
    Ok((CPU_QUOTA_PER_SEC_US.to_string(), Value::U64(quota)))
}
