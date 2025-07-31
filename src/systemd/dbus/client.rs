// Copyright 2021-2023 Kata Contributors
// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use zbus::{Error as ZbusError, Result as ZbusResult};

use crate::systemd::dbus::error::{Error, Result};
use crate::systemd::dbus::proxy::systemd_manager_proxy;
use crate::systemd::props::{Value, ZbusProperty, ZbusPropertyRef};
use crate::systemd::{Property, NO_SUCH_UNIT, PIDS, UNIT_MODE_REPLACE};
use crate::CgroupPid;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SystemdClient {
    /// The name of the systemd unit (slice or scope)
    unit: String,
    props: Vec<Property>,
}

impl SystemdClient {
    pub fn new(unit: &str, props: Vec<Property>) -> Result<Self> {
        Ok(Self {
            unit: unit.to_string(),
            props,
        })
    }
}

impl SystemdClient {
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
                if let Value::ArrayU32(arr) = &mut prop.1 {
                    arr.push(pid.pid as u32);
                    return Ok(());
                }
                // Invalid type of PIDs
                return Err(Error::InvalidProperties);
            }
        }
        // If PIDS is not set, we create a new property.
        self.props
            .push((PIDS.to_string(), vec![pid.pid as u32].into()));
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
        if !self.props.iter().any(|(k, _)| k == PIDS) {
            return Err(Error::InvalidProperties);
        }

        let sys_proxy = systemd_manager_proxy()?;

        let props: Vec<ZbusProperty<'_>> = self
            .props
            .iter()
            .map(|(k, v)| (k.clone(), v.into()))
            .collect();
        let props_ref: Vec<ZbusPropertyRef<'_>> =
            props.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let props_ref_ref: Vec<&ZbusPropertyRef<'_>> = props_ref.iter().collect();

        sys_proxy.start_transient_unit(&self.unit, UNIT_MODE_REPLACE, &props_ref_ref, &[])?;

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
    pub fn set_properties(&mut self, properties: &[Property]) -> Result<()> {
        for new in properties {
            // Try to update the value first, if fails, append it.
            if let Some(existing) = self.props.iter_mut().find(|p| p.0 == new.0) {
                existing.1 = new.1.clone();
            } else {
                self.props.push((new.0.clone(), new.1.clone()));
            }
        }

        // The unit must exist before setting properties.
        if !self.exists() {
            return Ok(());
        }

        let sys_proxy = systemd_manager_proxy()?;

        let props: Vec<ZbusProperty<'_>> = properties
            .iter()
            .map(|(k, v)| (k.clone(), v.into()))
            .collect();
        let props_ref: Vec<ZbusPropertyRef<'_>> =
            props.iter().map(|(k, v)| (k.as_str(), v)).collect();
        let props_ref_ref: Vec<&ZbusPropertyRef<'_>> = props_ref.iter().collect();

        sys_proxy.set_unit_properties(&self.unit, true, &props_ref_ref)?;

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

#[cfg(test)]
pub mod tests {
    //! Unit tests for the SystemdClient
    //!
    //! Not sure why the tests are going to fail if we run them in
    //! parallel. Everything goes smoothly in serial.
    //!
    //! $ cargo test --package cgroups-rs --lib \
    //!   -- systemd::dbus::client::tests \
    //!   --show-output --test-threads=1

    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::thread::sleep;
    use std::time::Duration;

    use rand::distributions::Alphanumeric;
    use rand::Rng;

    use crate::fs::hierarchies;
    use crate::systemd::dbus::client::*;
    use crate::systemd::props::PropertiesBuilder;
    use crate::systemd::utils::expand_slice;
    use crate::systemd::{DEFAULT_DESCRIPTION, DESCRIPTION, PIDS};
    use crate::tests::{spawn_sleep_inf, spawn_yes, systemd_version};

    const TEST_SLICE: &str = "cgroupsrs-test.slice";

    fn test_unit() -> String {
        let rand_string: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(5)
            .map(char::from)
            .collect();
        format!("cri-pod{}.scope", rand_string)
    }

    #[macro_export]
    macro_rules! skip_if_no_systemd {
        () => {
            if $crate::tests::systemd_version().is_none() {
                eprintln!("Test skipped, no systemd?");
                return;
            }
        };
    }

    fn systemd_show(unit: &str) -> String {
        let output = Command::new("systemctl")
            .arg("show")
            .arg(unit)
            .output()
            .expect("Failed to execute systemctl show command");
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn start_default_cgroup(pid: CgroupPid, unit: &str) -> SystemdClient {
        let mut props = PropertiesBuilder::default_cgroup(TEST_SLICE, unit).build();
        props.push((PIDS.to_string(), vec![pid.pid as u32].into()));
        let cgroup = SystemdClient::new(unit, props).unwrap();
        // Stop the unit if it exists.
        cgroup.stop().unwrap();

        // Write the current process to the cgroup.
        cgroup.start().unwrap();
        cgroup.add_process(pid, "/").unwrap();
        cgroup
    }

    fn stop_cgroup(cgroup: &SystemdClient) {
        cgroup.stop().unwrap();
    }

    #[test]
    fn test_start() {
        skip_if_no_systemd!();

        let v2 = hierarchies::is_cgroup2_unified_mode();
        let unit = test_unit();
        let mut child = spawn_sleep_inf();
        let cgroup = start_default_cgroup(CgroupPid::from(child.id() as u64), &unit);

        let base = expand_slice(TEST_SLICE).unwrap();

        // Check if the cgroup exists in the filesystem
        let full_base = if v2 {
            format!("/sys/fs/cgroup/{}", base)
        } else {
            format!("/sys/fs/cgroup/memory/{}", base)
        };
        assert!(
            Path::new(&full_base).exists(),
            "Cgroup base path does not exist: {}",
            full_base
        );

        // PIDs
        let cgroup_procs_path = format!("{}/{}/cgroup.procs", full_base, &unit);
        for i in 0..5 {
            let content = fs::read_to_string(&cgroup_procs_path);
            if let Ok(content) = &content {
                if content.contains(&child.id().to_string()) {
                    break;
                }
            }
            // Retry attempts exhausted, resulting in failure
            if i == 4 {
                let content = content.as_ref().unwrap();
                assert!(
                    content.contains(&child.id().to_string()),
                    "Cgroup procs does not contain the child process ID"
                );
            }
            // Wait 500ms before next retrying
            sleep(Duration::from_millis(500));
        }

        // Check the unit from "systemctl show <unit>"
        let output = systemd_show(&cgroup.unit);

        // Slice
        assert!(
            output
                .lines()
                .any(|line| line == format!("Slice={}", TEST_SLICE)),
            "Slice not found"
        );
        // Delegate
        assert!(
            output.lines().any(|line| line == "Delegate=yes"),
            "Delegate not set"
        );
        // DelegateControllers
        // controllers: cpu cpuacct cpuset io blkio memory devices pids
        let controllers = output
            .lines()
            .find(|line| line.starts_with("DelegateControllers="))
            .map(|line| line.trim_start_matches("DelegateControllers="))
            .unwrap();
        let controllers = controllers.split(' ').collect::<Vec<&str>>();
        assert!(
            controllers.contains(&"cpu"),
            "DelegateControllers cpu not set"
        );
        assert!(
            controllers.contains(&"cpuset"),
            "DelegateControllers cpuset not set"
        );
        if v2 {
            assert!(
                controllers.contains(&"io"),
                "DelegateControllers io not set"
            );
        } else {
            assert!(
                controllers.contains(&"blkio"),
                "DelegateControllers blkio not set"
            );
        }
        assert!(
            controllers.contains(&"memory"),
            "DelegateControllers memory not set"
        );
        assert!(
            controllers.contains(&"pids"),
            "DelegateControllers pids not set"
        );

        // CPUAccounting
        assert!(
            output.lines().any(|line| line == "CPUAccounting=yes"),
            "CPUAccounting not set"
        );
        // IOAccounting for v2, and BlockIOAccounting for v1
        if v2 {
            assert!(
                output.lines().any(|line| line == "IOAccounting=yes"),
                "IOAccounting not set"
            );
        } else {
            assert!(
                output.lines().any(|line| line == "BlockIOAccounting=yes"),
                "BlockIOAccounting not set"
            );
        }
        // MemoryAccounting
        assert!(
            output.lines().any(|line| line == "MemoryAccounting=yes"),
            "MemoryAccounting not set"
        );
        // TasksAccounting
        assert!(
            output.lines().any(|line| line == "TasksAccounting=yes"),
            "TasksAccounting not set"
        );
        // ActiveState
        assert!(
            output.lines().any(|line| line == "ActiveState=active"),
            "Unit is not active"
        );

        stop_cgroup(&cgroup);
        child.wait().unwrap();
    }

    #[test]
    fn test_stop() {
        skip_if_no_systemd!();

        let unit = test_unit();
        let mut child = spawn_sleep_inf();
        let cgroup = start_default_cgroup(CgroupPid::from(child.id() as u64), &unit);

        // Check ActiveState: expected to be "active"
        let output = systemd_show(&cgroup.unit);
        assert!(
            output.lines().any(|line| line == "ActiveState=active"),
            "Unit is not active"
        );

        stop_cgroup(&cgroup);

        // Check ActiveState: expected to be "inactive"
        let output = systemd_show(&cgroup.unit);
        assert!(
            output.lines().any(|line| line == "ActiveState=inactive"),
            "Unit is not inactive"
        );

        child.wait().unwrap();
    }

    #[test]
    fn test_set_properties() {
        skip_if_no_systemd!();

        let unit = test_unit();
        let mut child = spawn_sleep_inf();
        let mut cgroup = start_default_cgroup(CgroupPid::from(child.id() as u64), &unit);

        let output = systemd_show(&cgroup.unit);
        assert!(
            output.lines().any(|line| line
                == format!(
                    "Description={} {}:{}",
                    DEFAULT_DESCRIPTION, TEST_SLICE, unit
                )),
            "Initial description not set correctly"
        );

        let properties = [(
            DESCRIPTION.to_string(),
            "kata-container1 description".into(),
        )];
        cgroup.set_properties(&properties).unwrap();
        assert!(cgroup
            .props
            .iter()
            .any(|(k, v)| { k == DESCRIPTION && v == &"kata-container1 description".into() }));

        let output = systemd_show(&cgroup.unit);
        assert!(
            output
                .lines()
                .any(|line| line == "Description=kata-container1 description"),
            "Updated description not set correctly"
        );

        stop_cgroup(&cgroup);
        child.wait().unwrap();
    }

    #[test]
    fn test_freeze_and_thaw() {
        skip_if_no_systemd!();

        let unit = test_unit();
        let mut child = spawn_yes();
        let cgroup = start_default_cgroup(CgroupPid::from(child.id() as u64), &unit);

        // Freeze the unit
        cgroup.freeze().unwrap();

        let pid = child.id() as u64;

        let stat_path = format!("/proc/{}/stat", pid);
        let content = fs::read_to_string(&stat_path).unwrap();
        // The process state is the third field, e.g.:
        // 1234 (bash) S 1233 ...
        //             ^
        let mut content_iter = content.split_whitespace();
        assert_eq!(
            content_iter.nth(2).unwrap(),
            "S",
            "Process should be in 'S' (sleeping) state after freezing"
        );

        // Thaw the unit
        cgroup.thaw().unwrap();

        // No more S now
        let content = fs::read_to_string(&stat_path).unwrap();
        let mut content_iter = content.split_whitespace();
        assert_ne!(
            content_iter.nth(2).unwrap(),
            "S",
            "Process should not be in 'S' (sleeping) state after thawing"
        );

        stop_cgroup(&cgroup);
        child.wait().unwrap();
    }

    #[test]
    fn test_systemd_version() {
        skip_if_no_systemd!();

        let unit = test_unit();
        let props = PropertiesBuilder::default_cgroup(TEST_SLICE, &unit).build();
        let cgroup = SystemdClient::new(&unit, props).unwrap();
        let version = cgroup.systemd_version().unwrap();

        let expected_version = systemd_version().unwrap();
        assert_eq!(version, expected_version, "Systemd version mismatch");
    }

    #[test]
    fn test_exists() {
        skip_if_no_systemd!();

        let unit = test_unit();
        let mut child = spawn_sleep_inf();
        let cgroup = start_default_cgroup(CgroupPid::from(child.id() as u64), &unit);

        assert!(cgroup.exists(), "Cgroup should exist after starting");

        stop_cgroup(&cgroup);
        child.wait().unwrap();
    }

    #[test]
    fn test_add_process() {
        skip_if_no_systemd!();

        let unit = test_unit();
        let mut child = spawn_sleep_inf();
        let cgroup = start_default_cgroup(CgroupPid::from(child.id() as u64), &unit);

        let mut child1 = spawn_sleep_inf();
        let pid1 = CgroupPid::from(child1.id() as u64);
        cgroup.add_process(pid1, "/").unwrap();

        let cgroup_procs_path = format!(
            "/sys/fs/cgroup/{}/{}/cgroup.procs",
            expand_slice(TEST_SLICE).unwrap(),
            unit
        );
        for i in 0..5 {
            let content = fs::read_to_string(&cgroup_procs_path);
            if let Ok(content) = content {
                assert!(
                    content.contains(&child1.id().to_string()),
                    "Cgroup procs does not contain the child1 process ID"
                );
                break;
            }
            // Retry attempts exhausted, resulting in failure
            if i == 4 {
                content.unwrap();
            }
            // Wait 500ms before next retrying
            sleep(Duration::from_millis(500));
        }

        stop_cgroup(&cgroup);
        child.wait().unwrap();
        child1.wait().unwrap();
    }
}
