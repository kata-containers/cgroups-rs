// Copyright 2021-2023 Kata Contributors
// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use zbus::zvariant::Value;
use zbus::{Error as ZbusError, Result as ZbusResult};

use crate::systemd::dbus::error::{Error, Result};
use crate::systemd::dbus::proxy::systemd_manager_proxy;
use crate::systemd::{Property, NO_SUCH_UNIT, PIDS, UNIT_MODE_REPLACE};
use crate::CgroupPid;

pub struct SystemdClient<'a> {
    /// The name of the systemd unit (slice or scope)
    unit: String,
    props: Vec<Property<'a>>,
}

impl<'a> SystemdClient<'a> {
    pub fn new(unit: &str, props: Vec<Property<'a>>) -> Result<Self> {
        Ok(Self {
            unit: unit.to_string(),
            props,
        })
    }
}

impl SystemdClient<'_> {
    /// Set the pid to the PIDs property of the unit.
    ///
    /// Append a process ID to the PIDs property of the unit. If not
    /// exists, one property will be created.
    pub fn set_pid_prop(&mut self, pid: CgroupPid) -> Result<()> {
        if self.exists() {
            return Ok(());
        }

        for prop in self.props.iter_mut() {
            if prop.0 == PIDS {
                // If PIDS is already set, we append the new pid to the existing list.
                if let Value::Array(arr) = &mut prop.1 {
                    arr.append(pid.pid.into())
                        .map_err(|_| Error::InvalidProperties)?;
                    return Ok(());
                }
                // Invalid type of PIDs
                return Err(Error::InvalidProperties);
            }
        }
        // If PIDS is not set, we create a new property.
        self.props
            .push((PIDS, Value::Array(vec![pid.pid as u32].into())));
        Ok(())
    }

    /// Start a slice or a scope unit controlled and supervised by systemd.
    ///
    /// For more information, see:
    /// https://www.freedesktop.org/software/systemd/man/latest/systemd.unit.html
    /// https://www.freedesktop.org/software/systemd/man/latest/systemd.slice.html
    /// https://www.freedesktop.org/software/systemd/man/latest/systemd.scope.html
    pub fn start(&self) -> Result<()> {
        // PIDs property must be present
        if !self.props.iter().any(|(k, _)| k == &PIDS) {
            return Err(Error::InvalidProperties);
        }

        let sys_proxy = systemd_manager_proxy()?;

        let props_borrowed: Vec<(&str, &zbus::zvariant::Value)> =
            self.props.iter().map(|(k, v)| (*k, v)).collect();
        let props_borrowed: Vec<&(&str, &Value)> = props_borrowed.iter().collect();

        sys_proxy.start_transient_unit(&self.unit, UNIT_MODE_REPLACE, &props_borrowed, &[])?;

        Ok(())
    }

    /// Stop the current transient unit, the processes will be killed on
    /// unit stop, see [1].
    ///
    /// 1. https://www.freedesktop.org/software/systemd/man/latest/systemd.kill.html#KillMode=
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
    pub fn set_properties(&mut self, properties: &[Property<'static>]) -> Result<()> {
        for prop in properties {
            let new = prop.1.try_clone().map_err(|_| Error::InvalidProperties)?;
            // Try to update the value first, if fails, append it.
            if let Some(existing) = self.props.iter_mut().find(|p| p.0 == prop.0) {
                existing.1 = new;
            } else {
                self.props.push((prop.0, new));
            }
        }

        // The unit must exist before setting properties.
        if !self.exists() {
            return Ok(());
        }

        let sys_proxy = systemd_manager_proxy()?;

        let props_borrowed: Vec<(&str, &Value)> = properties.iter().map(|(k, v)| (*k, v)).collect();
        let props_borrowed: Vec<&(&str, &Value)> = props_borrowed.iter().collect();

        sys_proxy.set_unit_properties(&self.unit, true, &props_borrowed)?;

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
