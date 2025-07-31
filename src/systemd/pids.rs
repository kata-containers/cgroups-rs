// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::systemd::error::Result;
use crate::systemd::props::Value;
use crate::systemd::{Property, TASKS_MAX};

pub fn max(max: i64) -> Result<Property> {
    Ok((TASKS_MAX.to_string(), Value::U64(max as u64)))
}
