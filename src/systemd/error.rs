// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid argument")]
    InvalidArgument,

    #[error("obsolete systemd, please upgrade your systemd")]
    ObsoleteSystemd,

    #[error("resource not supported by cgroups v1")]
    CgroupsV1NotSupported,
}
