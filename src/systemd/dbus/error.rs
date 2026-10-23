// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0
//

use std::time::Duration;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid properties")]
    InvalidProperties,

    #[error("systemd job failed with result: {0}")]
    JobFailed(String),

    #[error("systemd JobRemoved signal stream closed")]
    JobRemovedSignalStreamClosed,

    #[error("timed out after {timeout:?} waiting for systemd job {job} for unit {unit}")]
    JobWaitTimedOut {
        unit: String,
        job: String,
        timeout: Duration,
    },

    #[error("systemd unit {unit} is stopping without a stop timeout")]
    UnitStopUnbounded { unit: String },

    #[error("systemd unit {unit} is inactive but can't be reused: {holders}")]
    UnitHeld { unit: String, holders: String },

    #[error("systemd unit signal stream closed")]
    UnitSignalStreamClosed,

    #[error("dbus error: {0}")]
    Dbus(#[from] zbus::Error),
}
