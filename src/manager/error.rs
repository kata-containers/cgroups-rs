// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use crate::fs::error::Error as CgroupfsError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("invalid argument")]
    InvalidArgument,

    #[error("invalid linux resource")]
    InvalidLinuxResource,

    #[error("cgroupfs error: {0}")]
    Cgroupfs(#[from] CgroupfsError),
}
