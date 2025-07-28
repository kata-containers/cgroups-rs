// Copyright (c) 2018 Levente Kurusa
// Copyright (c) 2020-2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

//! Systemd D-Bus interface for managing cgroups and units.
//!
//! References:
//! https://www.freedesktop.org/software/systemd/man/latest/org.freedesktop.systemd1.html
//! https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html
//! https://www.freedesktop.org/software/systemd/man/latest/systemd.resource-control.html

mod client;
pub mod error;
mod systemd_manager_proxy;
pub use client::SystemdClient;
mod proxy;
