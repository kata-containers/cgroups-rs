// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use std::collections::HashMap;

use oci_spec::runtime::{LinuxCpu, LinuxMemory, LinuxPids, LinuxResources};
use zbus::zvariant::Value as ZbusValue;

use crate::manager::conv;
use crate::manager::error::{Error, Result};
use crate::manager::fs::{join_path, FsManager};
use crate::systemd::props::PropertiesBuilder;
use crate::systemd::utils::expand_slice;
use crate::systemd::{
    cpu, cpuset, memory, pids, Property, SystemdClient, DEFAULT_SLICE, SCOPE_SUFFIX, SLICE_SUFFIX,
    TIMEOUT_STOP_USEC,
};
use crate::{CgroupPid, CgroupStats, FreezerState, Manager};

/// Default kernel value for cpu quota period is 100000 us (100 ms), same
/// for v1 [1] and v2 [2].
///
/// 1: https://www.kernel.org/doc/html/latest/scheduler/sched-bwc.html
/// 2: https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html
const DEFAULT_CPU_QUOTA_PERIOD: u64 = 100_000; // 100ms

pub struct SystemdManager<'a> {
    /// The name of slice
    slice: String,
    /// The name of unit
    unit: String,
    /// Systemd client
    systemd_client: SystemdClient<'a>,
    /// Cgroupfs manager
    fs_manager: FsManager,
}

impl SystemdManager<'_> {
    fn parse_slice_and_unit(path: &str) -> Result<(String, String)> {
        let parts: Vec<&str> = path.split(':').collect();
        if parts.len() != 3 {
            return Err(Error::InvalidArgument);
        }

        let slice = if parts[0].is_empty() {
            DEFAULT_SLICE.to_string()
        } else {
            parts[0].to_string()
        };

        let unit = new_unit_name(parts[1], parts[2]);

        Ok((slice, unit))
    }

    /// Create a new `SystemdManager` from a cgroup path.
    ///
    /// # Arguments
    ///
    /// * `path` - A string slice that holds the cgroup path in the format
    ///   "parent:scope_prefix:name".
    pub fn new(path: &str) -> Result<Self> {
        let (slice, unit) = Self::parse_slice_and_unit(path)?;
        let props = PropertiesBuilder::default_cgroup(&slice, &unit).build();
        let slice_base = expand_slice(&slice)?;

        let fs_base = join_path(&slice_base, &unit);
        let fs_manager = FsManager::new(&fs_base)?;

        let cgroup = SystemdClient::new(&unit, props)?;

        Ok(Self {
            slice,
            unit,
            fs_manager,
            systemd_client: cgroup,
        })
    }
}

impl SystemdManager<'_> {
    /// Get the slice name.
    pub fn slice(&self) -> &str {
        &self.slice
    }

    /// Get the unit name.
    pub fn unit(&self) -> &str {
        &self.unit
    }

    fn set_cpuset(
        &self,
        props: &mut Vec<Property>,
        linux_cpu: &LinuxCpu,
        systemd_version: usize,
    ) -> Result<()> {
        if let Some(cpus) = linux_cpu.cpus().as_ref() {
            let (id, value) = cpuset::cpus(cpus, systemd_version)?;
            props.push((id, value.into()));
        }

        if let Some(mems) = linux_cpu.mems().as_ref() {
            let (id, value) = cpuset::mems(mems, systemd_version)?;
            props.push((id, value.into()));
        }

        Ok(())
    }

    fn set_cpu(
        &self,
        props: &mut Vec<Property>,
        linux_cpu: &LinuxCpu,
        systemd_version: usize,
    ) -> Result<()> {
        if let Some(shares) = linux_cpu.shares() {
            let shares = if self.v2() {
                conv::cpu_shares_to_cgroup_v2(shares)
            } else {
                shares
            };
            let (id, value) = cpu::shares(shares, self.v2())?;
            props.push((id, value.into()));
        }

        let period = linux_cpu.period().unwrap_or(0);
        let quota = linux_cpu.quota().unwrap_or(0);

        if period != 0 {
            let (id, value) = cpu::period(period, systemd_version)?;
            props.push((id, value.into()));
        }

        if period != 0 || quota != 0 {
            // Corresponds to USEC_INFINITY in systemd
            let mut quota_systemd = u64::MAX;
            let mut period = period;
            if quota > 0 {
                if period == 0 {
                    period = DEFAULT_CPU_QUOTA_PERIOD;
                }
                // systemd converts CPUQuotaPerSecUSec (microseconds per
                // CPU second) to CPUQuota (integer percentage of CPU)
                // internally. This means that if a fractional percent of
                // CPU is indicated by Resources.CpuQuota, we need to round
                // up to the nearest 10ms (1% of a second) such that child
                // cgroups can set the cpu.cfs_quota_us they expect.
                quota_systemd = ((quota as u64) * s_to_us(1)) / period;
                if quota_systemd % ms_to_us(10) != 0 {
                    quota_systemd = (quota_systemd / ms_to_us(10) + 1) * ms_to_us(10);
                }
            }
            let (id, value) = cpu::quota(quota_systemd)?;
            props.push((id, value.into()));
        }

        Ok(())
    }

    fn set_memory(&self, props: &mut Vec<Property>, linux_memory: &LinuxMemory) -> Result<()> {
        let v2 = self.v2();

        let mem_limit = linux_memory.limit().unwrap_or(0);
        if mem_limit != 0 {
            let (id, value) = memory::limit(mem_limit, v2)?;
            props.push((id, value.into()));
        }

        let reservation = linux_memory.reservation().unwrap_or(0);
        if reservation != 0 && v2 {
            let (id, value) = memory::low(reservation, v2)?;
            props.push((id, value.into()));
        }

        let memswap_limit = linux_memory.swap().unwrap_or(0);
        if memswap_limit != 0 && v2 {
            let memswap_limit = conv::memory_swap_to_cgroup_v2(memswap_limit, mem_limit)?;
            let (id, value) = memory::swap(memswap_limit, v2)?;
            props.push((id, value.into()));
        }

        Ok(())
    }

    fn set_pids(&self, props: &mut Vec<Property>, linux_pids: &LinuxPids) -> Result<()> {
        let limit = linux_pids.limit();
        if limit == -1 || limit > 0 {
            let (id, value) = pids::max(limit)?;
            props.push((id, value.into()));
        }

        Ok(())
    }

    /// The systemd sends SIGTERM to processes in the unit on stop. Once a
    /// timeout occurs, SIGKILL will be sent to the processes.
    ///
    /// The item could be retrieved by:
    ///
    /// ```bash
    /// $ systemctl show <unit> -p TimeoutStopUSec
    /// ```
    pub fn set_term_timeout(&mut self, timeout_in_sec: u64) -> Result<()> {
        let timeout_in_usec = timeout_in_sec * 1_000_000;
        let prop = (TIMEOUT_STOP_USEC, ZbusValue::U64(timeout_in_usec));
        self.systemd_client.set_properties(&[prop])?;
        Ok(())
    }
}

impl Manager for SystemdManager<'_> {
    fn add_proc(&mut self, pid: CgroupPid) -> Result<()> {
        if !self.systemd_client.exists() {
            self.systemd_client.set_pid_prop(pid)?;
            self.systemd_client.start()?;
            // The fs_manager was created in load mode, which doesn't create
            // the cgroups. So we create them here.
            self.fs_manager.create_cgroups()?;
            return Ok(());
        }

        let subcgroup = self.fs_manager.subcgroup();
        self.systemd_client.add_process(pid, subcgroup)?;

        Ok(())
    }

    /// `add_thread()` is the same as `add_proc()`, as systemd doesn't
    /// expose an API to add a thread directly. As a result, the whole
    /// threads belonging to one process will be added to this cgroup.
    fn add_thread(&mut self, pid: CgroupPid) -> Result<()> {
        self.add_proc(pid)
    }

    fn cgroup_path(&self, subsystem: Option<&str>) -> Result<String> {
        self.fs_manager.cgroup_path(subsystem)
    }

    /// Destroy the cgroup and stop the transient unit.
    ///
    /// Please note that if the current manager is in the cgroup, the
    /// manager will be killed with SIGTERM signal. If you do not intend
    /// that, please ignore the signal and do cleanup things immediately.
    /// Systemd will forcibly terminate the process with SIGKILL after a
    /// while.
    fn destroy(&mut self) -> Result<()> {
        self.systemd_client.stop()?;
        Ok(())
    }

    fn enable_cpus_topdown(&self, cpus: &str) -> Result<()> {
        self.fs_manager.enable_cpus_topdown(cpus)
    }

    fn freeze(&self, state: FreezerState) -> Result<()> {
        match state {
            FreezerState::Thawed => self.systemd_client.thaw()?,
            FreezerState::Frozen => self.systemd_client.freeze()?,
            FreezerState::Freezing => return Err(Error::InvalidArgument),
        }

        Ok(())
    }

    fn pids(&self) -> Result<Vec<CgroupPid>> {
        self.fs_manager.pids()
    }

    fn set(&mut self, resources: &LinuxResources) -> Result<()> {
        let mut props = vec![];

        let systemd_version = self.systemd_client.systemd_version()?;

        if let Some(linux_cpu) = resources.cpu() {
            self.set_cpuset(&mut props, linux_cpu, systemd_version)?;
            self.set_cpu(&mut props, linux_cpu, systemd_version)?;
        }

        if let Some(linux_memory) = resources.memory() {
            self.set_memory(&mut props, linux_memory)?;
        }

        if let Some(linux_pids) = resources.pids() {
            self.set_pids(&mut props, linux_pids)?;
        }

        self.systemd_client.set_properties(&props)?;

        Ok(())
    }

    fn stats(&self) -> CgroupStats {
        self.fs_manager.stats()
    }

    fn paths(&self) -> &HashMap<String, String> {
        self.fs_manager.paths()
    }

    fn mounts(&self) -> &HashMap<String, String> {
        self.fs_manager.mounts()
    }

    fn systemd(&self) -> bool {
        true
    }

    fn v2(&self) -> bool {
        self.fs_manager.v2()
    }
}

fn new_unit_name(scope_prefix: &str, name: &str) -> String {
    // By default, we create a scope unless the user explicitly asks
    // for a slice.
    if !name.ends_with(SLICE_SUFFIX) {
        if scope_prefix.is_empty() {
            // {name}.scope
            return format!("{}{}", name, SCOPE_SUFFIX);
        }
        // {scope_prefix}-{name}.scope
        return format!("{}-{}{}", scope_prefix, name, SCOPE_SUFFIX);
    }

    name.to_string()
}

#[inline]
/// Convert milliseconds to microseconds.
fn ms_to_us(ms: u64) -> u64 {
    ms * 1_000
}

#[inline]
/// Convert seconds to microseconds.
fn s_to_us(s: u64) -> u64 {
    s * 1_000_000
}

#[cfg(test)]
mod tests {
    //! Tests for the `SystemdManager` implementation of the `Manager`
    //! trait.
    //!
    //! Don't run tests in parallel, use `--test-threads=1`!
    //!

    use std::path::Path;
    use std::thread::sleep;
    use std::time::Duration;

    use oci_spec::runtime::{LinuxCpuBuilder, LinuxMemoryBuilder, LinuxResourcesBuilder};
    use rand::distributions::Alphanumeric;
    use rand::Rng;

    use crate::fs::cpu::CpuController;
    use crate::fs::memory::MemController;
    use crate::fs::{ControllIdentifier, Controller, Subsystem};
    use crate::manager::systemd::*;
    use crate::manager::tests::{MEMORY_1G, MEMORY_2G, MEMORY_512M};
    use crate::tests::spawn_sleep_inf;
    use crate::{skip_if_cgroups_v1, skip_if_cgroups_v2, skip_if_no_systemd};

    fn new_cgroups_path() -> (String, String, String) {
        let rand_string: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(5)
            .map(char::from)
            .collect();
        (
            "cgroupsrs-test.slice".to_string(),
            "cri".to_string(),
            format!("pod{}", rand_string),
        )
    }

    fn new_systemd_manager<'a>() -> SystemdManager<'a> {
        let (slice, scope_prefix, name) = new_cgroups_path();
        SystemdManager::new(&format!("{}:{}:{}", slice, scope_prefix, name)).unwrap()
    }

    fn run_set_resources_failed(resources: LinuxResources) {
        let mut child = spawn_sleep_inf();
        let mut manager = new_systemd_manager();
        manager
            .add_proc(CgroupPid {
                pid: child.id() as u64,
            })
            .unwrap();
        assert!(manager.set(&resources).is_err());
        manager.destroy().unwrap();
        child.wait().unwrap();
    }

    fn run_set_resources<F>(linux_resources: LinuxResources, test_fn: F)
    where
        F: FnOnce(&mut SystemdManager),
    {
        let mut manager = new_systemd_manager();
        let mut child = spawn_sleep_inf();

        manager
            .add_proc(CgroupPid {
                pid: child.id() as u64,
            })
            .unwrap();
        manager.set(&linux_resources).unwrap();

        test_fn(&mut manager);

        manager.destroy().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn test_new_unit_name() {
        assert_eq!(new_unit_name("test", "unit"), "test-unit.scope");
        assert_eq!(new_unit_name("test", "unit.slice"), "unit.slice");
        assert_eq!(new_unit_name("", "unit"), "unit.scope");
        assert_eq!(new_unit_name("", "unit.slice"), "unit.slice");
        assert_eq!(new_unit_name("prefix", "unit"), "prefix-unit.scope");
    }

    #[test]
    fn test_slice_and_unit() {
        skip_if_no_systemd!();

        let (slice, scope_prefix, name) = new_cgroups_path();
        let manager = SystemdManager::new(&format!("{}:{}:{}", slice, scope_prefix, name)).unwrap();

        assert_eq!(manager.slice(), "cgroupsrs-test.slice");
        assert_eq!(manager.unit(), format!("{scope_prefix}-{name}.scope"));
    }

    #[test]
    fn test_destory() {
        skip_if_no_systemd!();

        let (slice, scope_prefix, name) = new_cgroups_path();
        let mut manager =
            SystemdManager::new(&format!("{}:{}:{}", slice, scope_prefix, name)).unwrap();

        let cgroup_path = manager.cgroup_path(Some("memory")).unwrap();
        // Before starting the unit, no cgroup should exist.
        assert!(!Path::new(&cgroup_path).exists());

        let mut child = spawn_sleep_inf();
        manager
            .add_proc(CgroupPid {
                pid: child.id() as u64,
            })
            .unwrap();

        // Now cgroup should exist.
        assert!(Path::new(&cgroup_path).exists());

        manager.destroy().unwrap();

        // This process should be killed.
        child.wait().unwrap();

        // No cgroup should exist after destroy, retry 5 times at 1-second
        // intervals.
        for _ in 0..5 {
            if !Path::new(&cgroup_path).exists() {
                break;
            }
            sleep(Duration::from_secs(1));
        }
        assert!(!Path::new(&cgroup_path).exists());
        // Unit should be stopped.
        assert!(!manager.systemd_client.exists());
    }

    fn controller<'a, T>(fs_manager: &'a FsManager) -> &'a T
    where
        &'a T: From<&'a Subsystem>,
        T: Controller + ControllIdentifier,
    {
        let controller: &T = fs_manager.cgroup().controller_of().unwrap();

        controller
    }

    #[test]
    fn test_set_cpu() {
        skip_if_no_systemd!();

        // 1024 shares, every 100ms allows to use 1 CPU
        let linux_cpu = LinuxCpuBuilder::default()
            .shares(1024u64)
            .quota(100000i64)
            .period(100000u64)
            .quota(100000i64)
            .build()
            .unwrap();

        let linux_resources = LinuxResourcesBuilder::default()
            .cpu(linux_cpu)
            .build()
            .unwrap();

        run_set_resources(linux_resources, |manager| {
            let controller: &CpuController = controller(&manager.fs_manager);
            let shares = controller.shares().unwrap();
            let period = controller.cfs_period().unwrap();
            let quota = controller.cfs_quota().unwrap();

            if manager.v2() {
                assert_eq!(shares, conv::cpu_shares_to_cgroup_v2(1024));
            } else {
                assert_eq!(shares, 1024);
            }

            assert_eq!(period, 100000);
            assert_eq!(quota, 100000);
        })
    }

    #[test]
    fn test_set_memory_v2() {
        skip_if_no_systemd!();
        skip_if_cgroups_v1!();

        // Expected failure: swap < limit
        let linux_memory = LinuxMemoryBuilder::default()
            .limit(MEMORY_1G)
            .swap(MEMORY_512M)
            .build()
            .unwrap();
        let linux_resources = LinuxResourcesBuilder::default()
            .memory(linux_memory)
            .build()
            .unwrap();
        run_set_resources_failed(linux_resources);

        // Expected success
        let linux_memory = LinuxMemoryBuilder::default()
            .limit(MEMORY_512M)
            .swap(MEMORY_1G)
            .reservation(MEMORY_2G)
            .build()
            .unwrap();
        let linux_resources = LinuxResourcesBuilder::default()
            .memory(linux_memory)
            .build()
            .unwrap();
        run_set_resources(linux_resources, |manager| {
            let controller: &MemController = controller(&manager.fs_manager);
            let memory_stat = controller.memory_stat();
            let memory_swap_stat = controller.memswap();

            assert_eq!(memory_stat.limit_in_bytes, MEMORY_512M);
            assert_eq!(memory_swap_stat.limit_in_bytes, MEMORY_512M);
            assert_eq!(memory_stat.soft_limit_in_bytes, MEMORY_2G);
        });
    }

    #[test]
    fn test_set_memory_v1() {
        skip_if_no_systemd!();
        skip_if_cgroups_v2!();

        // Expected success
        let linux_memory = LinuxMemoryBuilder::default()
            .limit(MEMORY_512M)
            .build()
            .unwrap();
        let linux_resources = LinuxResourcesBuilder::default()
            .memory(linux_memory)
            .build()
            .unwrap();
        run_set_resources(linux_resources, |manager| {
            let controller: &MemController = controller(&manager.fs_manager);
            let memory_stat = controller.memory_stat();
            assert_eq!(memory_stat.limit_in_bytes, MEMORY_512M);
        });
    }
}
