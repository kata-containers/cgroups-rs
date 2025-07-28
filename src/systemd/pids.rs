// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::systemd::error::Result;
use crate::systemd::TASKS_MAX;

pub fn max(max: i64) -> Result<(&'static str, u64)> {
    Ok((TASKS_MAX, max as u64))
}
