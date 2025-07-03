// Copyright 2021-2023 Kata Contributors
// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use libc::SIGKILL;
use zbus::zvariant::Value;
use zbus::{Error as ZbusError, Result as ZbusResult};

use crate::systemd::dbus::error::{Error, Result};
use crate::systemd::dbus::proxy::systemd_manager_proxy;
use crate::systemd::dbus::{
    DEFAULT_DEPENDENCIES, DELEGATE, DESCRIPTION, NO_SUCH_UNIT, PIDS, SLICE, UNIT_MODE_REPLACE,
    WANTS, WHO_ENUM_ALL,
};
use crate::systemd::{cgroup_properties, utils, Property};
use crate::CgroupPid;

pub struct SystemdClient {
    /// The name of slice
    slice: String,
    /// The name of the systemd unit (slice or scope)
    unit: String,
}

impl SystemdClient {
    pub fn new(slice: &str, unit: &str) -> Result<Self> {
        Ok(Self {
            slice: slice.to_string(),
            unit: unit.to_string(),
        })
    }
}

impl SystemdClient {
    /// Start a slice or a scope unit controlled and supervised by systemd.
    ///
    /// For more information, see:
    /// https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html
    /// https://www.freedesktop.org/software/systemd/man/latest/systemd.slice.html
    /// https://www.freedesktop.org/software/systemd/man/latest/systemd.scope.html
    pub fn start(&self, pid: CgroupPid) -> Result<()> {
        let mut props: Vec<Property> = vec![
            (DEFAULT_DEPENDENCIES, Value::Bool(false)),
            (DESCRIPTION, Value::Str("kata-containers unit".into())),
            (PIDS, Value::Array(vec![pid.pid as u32].into())),
        ];

        props.extend(cgroup_properties());

        if utils::is_slice_unit(&self.unit) {
            // If we create a slice, the parent is defined via a Wants=.
            props.push((WANTS, Value::Str(self.slice.as_str().into())));
        } else {
            // Otherwise it's a scope, which we put into a Slice=.
            props.push((SLICE, Value::Str(self.slice.as_str().into())));
            // Assume scopes always support delegation (supported since systemd v218).
            props.push((DELEGATE, Value::Bool(true)));
        }

        let sys_proxy = systemd_manager_proxy()?;

        let props_borrowed: Vec<(&str, &zbus::zvariant::Value)> =
            props.iter().map(|(k, v)| (*k, v)).collect();
        let props_borrowed: Vec<&(&str, &Value)> = props_borrowed.iter().collect();

        sys_proxy.start_transient_unit(&self.unit, UNIT_MODE_REPLACE, &props_borrowed, &[])?;

        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        let sys_proxy = systemd_manager_proxy()?;

        let ret = sys_proxy.stop_unit(&self.unit, UNIT_MODE_REPLACE);
        ignore_no_such_unit(ret)?;

        // If we stop the unit and it still exists, it may be in a failed
        // state, so we will try to reset it.
        if self.exists() {
            let ret = sys_proxy.reset_failed_unit(&self.unit);
            ignore_no_such_unit(ret)?;
        }

        Ok(())
    }

    /// Set properties for the unit through dbus `SetUnitProperties`.
    pub fn set_properties(&self, properties: &[Property]) -> Result<()> {
        let sys_proxy = systemd_manager_proxy()?;

        let props_borrowed: Vec<(&str, &Value)> = properties.iter().map(|(k, v)| (*k, v)).collect();
        let props_borrowed: Vec<&(&str, &Value)> = props_borrowed.iter().collect();

        sys_proxy.set_unit_properties(&self.unit, true, &props_borrowed)?;

        Ok(())
    }

    /// Kill the unit through dbus `KillUnit` with `SIGKILL` signal.
    pub fn kill(&self) -> Result<()> {
        let sys_proxy = systemd_manager_proxy()?;

        let ret = sys_proxy.kill_unit(&self.unit, WHO_ENUM_ALL, SIGKILL);
        ignore_no_such_unit(ret)?;

        Ok(())
    }

    /// Freeze the unit through dbus `FreezeUnit`.
    pub fn freeze(&self) -> Result<()> {
        let sys_proxy = systemd_manager_proxy()?;

        sys_proxy.freeze_unit(&self.unit)?;

        Ok(())
    }

    /// Thaw the frozen unit through dbus `ThawUnit`.
    pub fn thaw(&self) -> Result<()> {
        let sys_proxy = systemd_manager_proxy()?;

        sys_proxy.thaw_unit(&self.unit)?;

        Ok(())
    }

    /// Get the systemd version.
    pub fn systemd_version(&self) -> Result<usize> {
        let sys_proxy = systemd_manager_proxy()?;

        // Parse 249 from "249.11-0ubuntu3.16"
        let version = sys_proxy.version()?;
        let version = version
            .split('.')
            .next()
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or(Error::CorruptedSystemdVersion(version))?;

        Ok(version)
    }

    /// Check if the unit exists.
    pub fn exists(&self) -> bool {
        let sys_proxy = match systemd_manager_proxy() {
            Ok(proxy) => proxy,
            _ => return false,
        };

        sys_proxy
            .get_unit(&self.unit)
            .map(|_| true)
            .unwrap_or_default()
    }

    /// Add a process (tgid) to the unit through dbus
    /// `AttachProcessesToUnit`.
    pub fn add_process(&self, pid: CgroupPid, subcgroup: &str) -> Result<()> {
        let sys_proxy = systemd_manager_proxy()?;

        sys_proxy.attach_processes_to_unit(&self.unit, subcgroup, &[pid.pid as u32])?;

        Ok(())
    }
}

fn ignore_no_such_unit<T>(result: ZbusResult<T>) -> ZbusResult<bool> {
    if let Err(ZbusError::MethodError(err_name, _, _)) = &result {
        if err_name.as_str() == NO_SUCH_UNIT {
            return Ok(true);
        }
    }
    result.map(|_| false)
}
