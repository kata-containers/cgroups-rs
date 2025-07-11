// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::systemd::dbus;
use crate::systemd::error::Result;

pub fn max(max: i64) -> Result<(&'static str, u64)> {
    Ok((dbus::TASKS_MAX, max as u64))
}
