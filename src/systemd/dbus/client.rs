// Copyright 2021-2023 Kata Contributors
// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use std::convert::TryFrom;
use std::future::Future;
use std::time::Duration;

use async_io::Timer;
use futures_lite::{future, StreamExt};
use log::warn;
use zbus::fdo::PropertiesProxy;
use zbus::names::InterfaceName;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Error as ZbusError, Result as ZbusResult};

use crate::systemd::dbus::error::{Error, Result};
use crate::systemd::dbus::proxy::systemd_manager_proxy;
use crate::systemd::dbus::systemd_manager_proxy::ManagerProxy as AsyncSystemManager;
use crate::systemd::utils::is_scope_unit;
use crate::systemd::{
    Property, ACTIVE_STATE, DEACTIVATING, FAILED, INACTIVE, JOB, NO_SUCH_UNIT, PIDS, REFS,
    SCOPE_INTERFACE, SYSTEMD_SERVICE, TIMEOUT_STOP_USEC, UNIT_INTERFACE, UNIT_MODE_REPLACE,
};
use crate::CgroupPid;

const START_JOB_TIMEOUT: Duration = Duration::from_secs(30);

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

        self.add_pid_prop(pid)
    }

    /// Same as `set_pid_prop()`, for a caller that knows the unit doesn't
    /// exist.
    pub(crate) fn add_pid_prop(&mut self, pid: CgroupPid) -> Result<()> {
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

        let props_borrowed: Vec<(&str, &zbus::zvariant::Value)> =
            self.props.iter().map(|(k, v)| (*k, v)).collect();
        let props_borrowed: Vec<&(&str, &Value)> = props_borrowed.iter().collect();

        async_io::block_on(async {
            let connection = zbus::Connection::system().await?;
            let sys_proxy = AsyncSystemManager::new(&connection).await?;

            // StartTransientUnit only queues the start job. Install the signal
            // listener first so a fast job cannot finish before we start waiting.
            let mut job_removed = sys_proxy
                .receive_job_removed_with_args(&[(2, self.unit.as_str())])
                .await?;
            sys_proxy.subscribe().await?;

            let job = sys_proxy
                .start_transient_unit(&self.unit, UNIT_MODE_REPLACE, &props_borrowed, &[])
                .await?;
            let job_path = job.as_str().to_string();
            let expected_job_path = job_path.clone();

            let wait_for_job = async move {
                while let Some(signal) = job_removed.next().await {
                    let args = signal.args()?;
                    if args.job.as_str() != expected_job_path.as_str() {
                        continue;
                    }

                    return match args.result {
                        "done" => Ok(()),
                        result => Err(Error::JobFailed(result.to_string())),
                    };
                }

                Err(Error::JobRemovedSignalStreamClosed)
            };
            let timeout_error = Error::JobWaitTimedOut {
                unit: self.unit.clone(),
                job: job_path,
                timeout: START_JOB_TIMEOUT,
            };

            with_timeout(wait_for_job, START_JOB_TIMEOUT, timeout_error).await
        })
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

        // A leftover unit with the same name may still be loaded, the
        // properties are then applied when the new unit is started.
        if !self.is_running()? {
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

    /// Check if the unit exists and isn't stopping, failed or inactive. A
    /// queued stop job may not have started yet.
    fn is_running(&self) -> Result<bool> {
        async_io::block_on(async {
            let connection = zbus::Connection::system().await?;
            let sys_proxy = AsyncSystemManager::new(&connection).await?;

            let path = match get_unit(&sys_proxy, &self.unit).await? {
                Some(path) => path,
                None => return Ok(false),
            };
            let unit = unit_properties(&connection, path).await?;
            let state: String = property(&unit, UNIT_INTERFACE, ACTIVE_STATE).await?;

            if matches!(state.as_str(), INACTIVE | FAILED | DEACTIVATING) {
                return Ok(false);
            }

            Ok(!has_job(&unit).await?)
        })
    }

    /// systemd refuses to attach processes to a unit that is stopping or
    /// failed, and to start a new transient unit with its name. Returns
    /// whether a running unit with this name exists.
    ///
    /// The wait has no deadline, as it only lasts while systemd is bound to
    /// end it. A leftover unit that may never go away is an error.
    pub fn wait_stale_unit_gone(&self) -> Result<bool> {
        async_io::block_on(async {
            let connection = zbus::Connection::system().await?;
            let sys_proxy = AsyncSystemManager::new(&connection).await?;

            let path = match get_unit(&sys_proxy, &self.unit).await? {
                Some(path) => path,
                None => return Ok(false),
            };
            let unit = unit_properties(&connection, path).await?;

            // Install the listeners before reading the state, so a change
            // right after the read is not missed.
            let mut removed = sys_proxy
                .receive_unit_removed_with_args(&[(0, self.unit.as_str())])
                .await?;
            let mut changed = unit
                .receive_properties_changed_with_args(&[(0, UNIT_INTERFACE)])
                .await?
                .filter(|signal| match signal.args() {
                    Ok(args) => {
                        let changed = args.changed_properties();
                        changed.contains_key(ACTIVE_STATE) || changed.contains_key(JOB)
                    }
                    Err(_) => true,
                });
            sys_proxy.subscribe().await?;

            let mut waiting = false;
            loop {
                if get_unit(&sys_proxy, &self.unit).await?.is_none() {
                    return Ok(false);
                }

                let state: String = property(&unit, UNIT_INTERFACE, ACTIVE_STATE).await?;
                match state.as_str() {
                    DEACTIVATING => self.check_stop_bounded(&unit).await?,
                    FAILED => {
                        ignore_no_such_unit(sys_proxy.reset_failed_unit(&self.unit).await)?;
                    }
                    INACTIVE => self.check_not_held(&sys_proxy, &unit).await?,
                    // A queued stop job may not have started yet.
                    _ if has_job(&unit).await? => {}
                    _ => return Ok(true),
                }

                if !waiting {
                    warn!(
                        "waiting for the leftover systemd unit {} ({}) to go away",
                        self.unit, state
                    );
                    waiting = true;
                }

                let next = future::or(async { removed.next().await.map(|_| ()) }, async {
                    changed.next().await.map(|_| ())
                });
                if next.await.is_none() {
                    return Err(Error::UnitSignalStreamClosed);
                }
            }
        })
    }

    /// Once its stop timeout expires, systemd kills the processes of a
    /// stopping scope, or gives up on them.
    async fn check_stop_bounded(&self, unit: &PropertiesProxy<'_>) -> Result<()> {
        if !is_scope_unit(&self.unit) {
            return Ok(());
        }

        let timeout: u64 = property(unit, SCOPE_INTERFACE, TIMEOUT_STOP_USEC).await?;
        if timeout == u64::MAX {
            return Err(Error::UnitStopUnbounded {
                unit: self.unit.clone(),
            });
        }

        Ok(())
    }

    /// systemd unloads an inactive unit before it handles our next message,
    /// unless something holds it. A pending job ends on its own, but
    /// leftover processes or D-Bus references may never go away.
    async fn check_not_held(
        &self,
        sys_proxy: &AsyncSystemManager<'_>,
        unit: &PropertiesProxy<'_>,
    ) -> Result<()> {
        if has_job(unit).await? {
            return Ok(());
        }

        let mut holders = Vec::new();

        let refs: Vec<String> = property(unit, UNIT_INTERFACE, REFS).await?;
        if !refs.is_empty() {
            holders.push(format!("referenced by D-Bus clients {}", refs.join(", ")));
        }

        let processes = match sys_proxy.get_unit_processes(&self.unit).await {
            Ok(processes) => processes,
            Err(ZbusError::MethodError(name, _, _)) if name.as_str() == NO_SUCH_UNIT => {
                return Ok(());
            }
            Err(err) => return Err(err.into()),
        };
        if !processes.is_empty() {
            let processes: Vec<String> = processes
                .iter()
                .map(|(_, pid, command)| format!("{} ({})", pid, command))
                .collect();
            holders.push(format!(
                "processes left in its cgroup: {}",
                processes.join(", ")
            ));
        }

        if holders.is_empty() {
            return Ok(());
        }

        Err(Error::UnitHeld {
            unit: self.unit.clone(),
            holders: holders.join("; "),
        })
    }

    /// Add a process (tgid) to the unit through dbus
    /// `AttachProcessesToUnit`.
    pub fn add_process(&self, pid: CgroupPid, subcgroup: &str) -> Result<()> {
        let sys_proxy = systemd_manager_proxy()?;

        sys_proxy.attach_processes_to_unit(&self.unit, subcgroup, &[pid.pid as u32])?;

        Ok(())
    }
}

async fn with_timeout<T, F>(operation: F, timeout: Duration, timeout_error: Error) -> Result<T>
where
    F: Future<Output = Result<T>>,
{
    future::race(operation, async move {
        Timer::after(timeout).await;
        Err(timeout_error)
    })
    .await
}

async fn get_unit(
    sys_proxy: &AsyncSystemManager<'_>,
    name: &str,
) -> Result<Option<OwnedObjectPath>> {
    match sys_proxy.get_unit(name).await {
        Ok(path) => Ok(Some(path)),
        Err(ZbusError::MethodError(err_name, _, _)) if err_name.as_str() == NO_SUCH_UNIT => {
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

async fn unit_properties(
    connection: &zbus::Connection,
    path: OwnedObjectPath,
) -> Result<PropertiesProxy<'static>> {
    Ok(PropertiesProxy::builder(connection)
        .destination(SYSTEMD_SERVICE)?
        .path(path)?
        .build()
        .await?)
}

async fn property<T>(unit: &PropertiesProxy<'_>, interface: &'static str, name: &str) -> Result<T>
where
    T: TryFrom<OwnedValue>,
    T::Error: Into<ZbusError>,
{
    let value = unit
        .get(InterfaceName::from_static_str_unchecked(interface), name)
        .await
        .map_err(ZbusError::from)?;

    T::try_from(value).map_err(|err| Error::Dbus(err.into()))
}

async fn has_job(unit: &PropertiesProxy<'_>) -> Result<bool> {
    let (id, _): (u32, OwnedObjectPath) = property(unit, UNIT_INTERFACE, JOB).await?;
    Ok(id != 0)
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
    use crate::systemd::dbus::error::Error;
    use crate::systemd::props::PropertiesBuilder;
    use crate::systemd::utils::expand_slice;
    use crate::systemd::{DEFAULT_DESCRIPTION, DESCRIPTION, PIDS, TIMEOUT_STOP_USEC};
    use crate::tests::{spawn_sleep_inf, spawn_term_ignoring, spawn_yes};

    const TEST_SLICE: &str = "cgroupsrs-test.slice";

    #[test]
    fn test_wait_timeout() {
        let timeout = Duration::from_millis(10);
        let result = async_io::block_on(with_timeout(
            futures_lite::future::pending::<crate::systemd::dbus::error::Result<()>>(),
            timeout,
            Error::JobWaitTimedOut {
                unit: "test.scope".to_string(),
                job: "/org/freedesktop/systemd1/job/1".to_string(),
                timeout,
            },
        ));

        match result {
            Err(Error::JobWaitTimedOut {
                unit,
                job,
                timeout: actual_timeout,
            }) => {
                assert_eq!(unit, "test.scope");
                assert_eq!(job, "/org/freedesktop/systemd1/job/1");
                assert_eq!(actual_timeout, timeout);
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }

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

    fn start_default_cgroup(pid: CgroupPid, unit: &'_ str) -> SystemdClient<'_> {
        start_cgroup(pid, unit, vec![])
    }

    fn start_cgroup<'a>(
        pid: CgroupPid,
        unit: &'a str,
        extra: Vec<Property<'a>>,
    ) -> SystemdClient<'a> {
        let mut props = PropertiesBuilder::default_cgroup(TEST_SLICE, unit).build();
        props.push((PIDS, Value::Array(vec![pid.pid as u32].into())));
        props.extend(extra);
        let cgroup = SystemdClient::new(unit, props).unwrap();
        // Stop the unit if it exists.
        cgroup.stop().unwrap();

        // Write the current process to the cgroup.
        cgroup.start().unwrap();
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
        // StopUnit only queues the stop job.
        cgroup.wait_stale_unit_gone().unwrap();

        // Check ActiveState: expected to be "inactive"
        let output = systemd_show(&cgroup.unit);
        assert!(
            output.lines().any(|line| line == "ActiveState=inactive"),
            "Unit is not inactive"
        );

        child.wait().unwrap();
    }

    #[test]
    fn test_wait_unit_held_by_ref() {
        skip_if_no_systemd!();

        let unit = test_unit();
        let mut child = spawn_sleep_inf();
        let cgroup = start_default_cgroup(CgroupPid::from(child.id() as u64), &unit);

        let sys_proxy = systemd_manager_proxy().unwrap();
        sys_proxy.ref_unit(&unit).unwrap();
        stop_cgroup(&cgroup);
        child.wait().unwrap();

        let err = cgroup.wait_stale_unit_gone().unwrap_err();
        assert!(matches!(err, Error::UnitHeld { .. }), "{}", err);

        sys_proxy.unref_unit(&unit).unwrap();
        assert!(!cgroup.wait_stale_unit_gone().unwrap());
    }

    #[test]
    fn test_wait_unit_held_by_processes() {
        skip_if_no_systemd!();

        // systemd gives up on stopping the unit, and leaves the process.
        let unit = test_unit();
        let mut child = spawn_term_ignoring();
        let cgroup = start_cgroup(
            CgroupPid::from(child.id() as u64),
            &unit,
            vec![
                (TIMEOUT_STOP_USEC, Value::U64(1_000_000)),
                ("SendSIGKILL", Value::Bool(false)),
            ],
        );
        stop_cgroup(&cgroup);

        let err = cgroup.wait_stale_unit_gone().unwrap_err();
        assert!(matches!(err, Error::UnitHeld { .. }), "{}", err);

        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!cgroup.wait_stale_unit_gone().unwrap());
    }

    #[test]
    fn test_wait_unit_stop_unbounded() {
        skip_if_no_systemd!();

        let unit = test_unit();
        let mut child = spawn_term_ignoring();
        let cgroup = start_cgroup(
            CgroupPid::from(child.id() as u64),
            &unit,
            vec![(TIMEOUT_STOP_USEC, Value::U64(u64::MAX))],
        );
        stop_cgroup(&cgroup);

        let err = cgroup.wait_stale_unit_gone().unwrap_err();
        assert!(matches!(err, Error::UnitStopUnbounded { .. }), "{}", err);

        child.kill().unwrap();
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
            DESCRIPTION,
            Value::Str("kata-container1 description".into()),
        )];
        cgroup.set_properties(&properties).unwrap();
        assert!(cgroup.props.iter().any(|(k, v)| {
            k == &DESCRIPTION && v == &Value::Str("kata-container1 description".into())
        }));

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

        let v2 = hierarchies::is_cgroup2_unified_mode();
        let unit = test_unit();
        let mut child = spawn_yes();
        let cgroup = start_default_cgroup(CgroupPid::from(child.id() as u64), &unit);

        // Freeze the unit
        let freeze_result = cgroup.freeze();
        if !v2 {
            assert!(
                freeze_result.is_err(),
                "systemd should reject FreezeUnit on cgroup v1"
            );
            stop_cgroup(&cgroup);
            child.wait().unwrap();
            return;
        }
        freeze_result.unwrap();

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

        let v2 = hierarchies::is_cgroup2_unified_mode();
        let unit = test_unit();
        let mut child = spawn_sleep_inf();
        let cgroup = start_default_cgroup(CgroupPid::from(child.id() as u64), &unit);

        let mut child1 = spawn_sleep_inf();
        let pid1 = CgroupPid::from(child1.id() as u64);
        cgroup.add_process(pid1, "/").unwrap();

        let cgroup_root = if v2 {
            Path::new("/sys/fs/cgroup")
        } else {
            Path::new("/sys/fs/cgroup/memory")
        };
        let cgroup_procs_path = cgroup_root
            .join(expand_slice(TEST_SLICE).unwrap())
            .join(&unit)
            .join("cgroup.procs");
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
