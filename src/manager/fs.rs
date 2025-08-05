// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use oci_spec::runtime::{
    LinuxBlockIo, LinuxCpu, LinuxDeviceCgroup, LinuxHugepageLimit, LinuxMemory, LinuxNetwork,
    LinuxPids, LinuxResources,
};
use serde::{Deserialize, Serialize};

use crate::fs::blkio::{BlkIoController, BlkIoData, IoService, IoStat};
use crate::fs::cgroup::UNIFIED_MOUNTPOINT;
use crate::fs::cpu::CpuController;
use crate::fs::cpuacct::CpuAcctController;
use crate::fs::cpuset::CpuSetController;
use crate::fs::devices::{DevicePermissions, DeviceType, DevicesController};
use crate::fs::error::{Error as FsError, ErrorKind as FsErrorKind, Result as FsResult};
use crate::fs::freezer::FreezerController;
use crate::fs::hugetlb::HugeTlbController;
use crate::fs::memory::MemController;
use crate::fs::net_cls::NetClsController;
use crate::fs::net_prio::NetPrioController;
use crate::fs::pid::PidController;
use crate::fs::{hierarchies, Cgroup, ControllIdentifier, Controller, MaxValue, Subsystem};
use crate::manager::error::Error;
use crate::manager::{conv, Manager, Result};
use crate::stats::{
    BlkioCgroupStats, BlkioStat, CpuAcctStats, CpuCgroupStats, CpuThrottlingStats,
    DeviceCgroupStat, DevicesCgroupStats, HugeTlbCgroupStats, HugeTlbStat, MemoryCgroupStats,
    MemoryStats, PidsCgroupStats,
};
use crate::{CgroupPid, CgroupStats, FreezerState};

const CGROUP_PATH: &str = "/proc/self/cgroup";
const MOUNTINFO_PATH: &str = "/proc/self/mountinfo";

/// FsManager manages cgroups using the cgroup filesystem (cgroupfs).
///
/// This manager deals with `LinuxResources` conformed to the OCI runtime
/// specification, so that it allows users not to do type conversions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsManager {
    /// Cgroup subsystem paths read from `/proc/self/cgroup`
    /// - cgroup v1: <subsystem> -> <path>
    /// - cgroup v2: "" -> <path>
    paths: HashMap<String, String>,
    /// Cgroup mountpoints read from `/proc/self/mountinfo`.
    mounts: HashMap<String, String>,
    /// Base path of the cgroup filesystem, the complete path would be:
    /// - cgroup v1: "/sys/fs/cgroup/<subsystem>/<base>"
    /// - cgroup v2: "/sys/fs/cgroup/<base>"
    base: String,
    /// Cgroup managed by this manager.
    #[serde(skip)]
    cgroup: Cgroup,
}

impl FsManager {
    /// Check if the cgroup exists or not.
    pub fn exists(&self) -> bool {
        self.cgroup.exists()
    }

    /// Create an instance of FsManager. The cgroups won't be created until
    /// `apply()` is called.
    pub fn new(base: &str) -> Result<Self> {
        let paths = parse_cgroup_subsystems()?;
        let mounts = parse_cgroup_mountinfo(&paths)?;
        let cgroup = Cgroup::load(hierarchies::auto(), base);
        let base = base.to_string();

        Ok(Self {
            paths,
            mounts,
            base,
            cgroup,
        })
    }
}

impl FsManager {
    /// Create the cgroups if they are not created yet.
    pub fn create_cgroups(&mut self) -> Result<()> {
        if self.exists() {
            return Ok(());
        }
        self.cgroup.create()?;
        Ok(())
    }

    /// Get the subcgroup path, which is useful for Docker-in-Docker (DinD)
    /// with cgroup v2, see [1].
    ///
    /// 1: https://github.com/kata-containers/kata-containers/issues/10733
    pub fn subcgroup(&self) -> &str {
        // Check if we're in a Docker-in-Docker setup by verifying:
        // 1. We're using cgroups v2 (which restricts direct process control)
        // 2. An "init" subdirectory exists (used by DinD for process
        //    delegation)
        let init_exists = hierarchies::auto()
            .root()
            .join(&self.base)
            .join("init")
            .exists();
        let is_dind = self.v2() && init_exists;

        if is_dind {
            "/init/"
        } else {
            "/"
        }
    }

    fn controller<'a, T>(&'a self) -> FsResult<&'a T>
    where
        &'a T: From<&'a Subsystem>,
        T: Controller + ControllIdentifier,
    {
        let controller: &T = self
            .cgroup
            .controller_of()
            .ok_or(FsError::new(FsErrorKind::SubsystemsEmpty))?;

        Ok(controller)
    }

    fn set_cpuset(&self, linux_cpu: &LinuxCpu) -> Result<()> {
        let controller: &CpuSetController = match self.controller() {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        if let Some(cpus) = linux_cpu.cpus() {
            controller.set_cpus(cpus)?;
        }

        if let Some(mems) = linux_cpu.mems() {
            controller.set_mems(mems)?;
        }

        Ok(())
    }

    fn set_cpu(&self, linux_cpu: &LinuxCpu) -> Result<()> {
        let controller: &CpuController = match self.controller() {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        if let Some(shares) = linux_cpu.shares() {
            let shares = if self.v2() {
                conv::cpu_shares_to_cgroup_v2(shares)
            } else {
                shares
            };
            if shares != 0 {
                controller.set_shares(shares)?;
            }
        }

        if let Some(quota) = linux_cpu.quota() {
            controller.set_cfs_quota(quota)?;
        }

        if let Some(period) = linux_cpu.period() {
            controller.set_cfs_period(period)?;
        }

        if let Some(rt_runtime) = linux_cpu.realtime_runtime() {
            controller.set_rt_runtime(rt_runtime)?;
        }

        if let Some(rt_period) = linux_cpu.realtime_period() {
            controller.set_rt_period_us(rt_period)?;
        }

        Ok(())
    }

    fn set_mem_and_memswap_v1(&self, limit: i64, mut swap_limit: i64) -> Result<()> {
        let controller: &MemController = self.controller()?;

        // If the memory update is set to -1 and the swap is not set
        // explicitly, we should also set swap to -1, it means
        // unlimited memory.
        if limit == -1 && swap_limit == 0 {
            swap_limit = -1;
        }

        if limit != 0 && swap_limit != 0 {
            let memory = controller.memory_stat();
            let limit_actual = memory.limit_in_bytes;

            // When update memory limit, we should adapt the write sequence
            // for memory and swap memory, so it won't fail because the new
            // value and the old value don't fit kernel's validation.
            if swap_limit == -1 || limit_actual < swap_limit {
                controller.set_memswap_limit(swap_limit)?;
                controller.set_limit(limit)?;

                return Ok(());
            }
        }

        if limit != 0 {
            controller.set_limit(limit)?;
        }
        if swap_limit != 0 {
            controller.set_memswap_limit(swap_limit)?;
        }

        Ok(())
    }

    fn set_memory_v1(&self, linux_memory: &LinuxMemory) -> Result<()> {
        let controller: &MemController = match self.controller() {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        let mem_limit = linux_memory.limit().unwrap_or(0);
        let memswap_limit = linux_memory.swap().unwrap_or(0);

        self.set_mem_and_memswap_v1(mem_limit, memswap_limit)?;

        if let Some(reservation) = linux_memory.reservation() {
            controller.set_soft_limit(reservation)?;
        }

        if linux_memory.disable_oom_killer().unwrap_or_default() {
            controller.disable_oom_killer()?;
        }

        if let Some(swappiness) = linux_memory.swappiness() {
            if swappiness <= 100 {
                controller.set_swappiness(swappiness)?;
            } else {
                return Err(Error::InvalidLinuxResource);
            };
        }

        Ok(())
    }

    fn set_memory_v2(&self, linux_memory: &LinuxMemory) -> Result<()> {
        let controller: &MemController = match self.controller() {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        if linux_memory.reservation().is_none()
            && linux_memory.limit().is_none()
            && linux_memory.swap().is_none()
        {
            return Ok(());
        }

        let mem_limit = linux_memory.limit().unwrap_or(0);
        let memswap_limit = linux_memory.swap().unwrap_or(0);

        // Check memory usage
        if mem_limit <= 0 && memswap_limit <= 0 {
            return Ok(());
        }

        let memory_stat = controller.memory_stat();
        let usage_actual = memory_stat.usage_in_bytes;

        // Rejecting: memory+swap limit <= usage
        if memswap_limit > 0 && memswap_limit as u64 <= usage_actual {
            return Err(Error::InvalidLinuxResource);
        }

        // Rejecting: memory limit <= usage
        if mem_limit > 0 && mem_limit as u64 <= usage_actual {
            return Err(Error::InvalidLinuxResource);
        }

        let swap_limit = conv::memory_swap_to_cgroup_v2(memswap_limit, mem_limit)?;
        controller.set_memswap_limit(swap_limit)?;

        if mem_limit != 0 {
            controller.set_limit(mem_limit)?;
        }

        if let Some(reservation) = linux_memory.reservation() {
            controller.set_soft_limit(reservation)?;
        }

        Ok(())
    }

    /// Set memory resources.
    ///
    /// Ignore kernel memory and kernel memory TCP, as runc does, see [1].
    ///
    /// 1: https://github.com/opencontainers/cgroups/blob/d36d371fe756a30d2e21d83c6b42e86af77bf4a2/fs/memory.go#L36
    fn set_memory(&self, linux_memory: &LinuxMemory) -> Result<()> {
        if self.v2() {
            self.set_memory_v2(linux_memory)?;
        } else {
            self.set_memory_v1(linux_memory)?;
        }

        Ok(())
    }

    fn set_pids(&self, pids: &LinuxPids) -> Result<()> {
        let controller: &PidController = match self.controller() {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        let value = if pids.limit() > 0 {
            MaxValue::Value(pids.limit())
        } else {
            MaxValue::Max
        };
        controller.set_pid_max(value)?;

        Ok(())
    }

    fn set_blkio(&self, blkio: &LinuxBlockIo) -> Result<()> {
        let controller: &BlkIoController = match self.controller() {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        if let Some(weight) = blkio.weight() {
            controller.set_weight(weight as u64)?;
        }

        if let Some(leaf_weight) = blkio.leaf_weight() {
            controller.set_leaf_weight(leaf_weight as u64)?;
        }

        if let Some(devices) = blkio.weight_device() {
            for device in devices.iter() {
                let major = device.major() as u64;
                let minor = device.minor() as u64;
                if let Some(weight) = device.weight() {
                    controller.set_weight_for_device(major, minor, weight as u64)?;
                }
                if let Some(leaf_weight) = device.leaf_weight() {
                    controller.set_leaf_weight_for_device(major, minor, leaf_weight as u64)?;
                }
            }
        }

        if let Some(devices) = blkio.throttle_read_bps_device() {
            for device in devices.iter() {
                let major = device.major() as u64;
                let minor = device.minor() as u64;
                let rate = device.rate();
                controller.throttle_read_bps_for_device(major, minor, rate)?;
            }
        }

        if let Some(devices) = blkio.throttle_write_bps_device() {
            for device in devices.iter() {
                let major = device.major() as u64;
                let minor = device.minor() as u64;
                let rate = device.rate();
                controller.throttle_write_bps_for_device(major, minor, rate)?;
            }
        }

        if let Some(devices) = blkio.throttle_read_iops_device() {
            for device in devices.iter() {
                let major = device.major() as u64;
                let minor = device.minor() as u64;
                let rate = device.rate();
                controller.throttle_read_iops_for_device(major, minor, rate)?;
            }
        }

        if let Some(devices) = blkio.throttle_write_iops_device() {
            for device in devices.iter() {
                let major = device.major() as u64;
                let minor = device.minor() as u64;
                let rate = device.rate();
                controller.throttle_write_iops_for_device(major, minor, rate)?;
            }
        }

        Ok(())
    }

    fn set_hugepages(&self, hugepage_limits: &[LinuxHugepageLimit]) -> Result<()> {
        let controller: &HugeTlbController = match self.controller() {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        for limit in hugepage_limits.iter() {
            // ignore not supported page size
            if !controller.size_supported(limit.page_size()) {
                continue;
            }
            let page_size = limit.page_size();
            let limit = limit.limit() as u64;
            controller.set_limit_in_bytes(page_size, limit)?;
        }

        Ok(())
    }

    fn set_network(&self, network: &LinuxNetwork) -> Result<()> {
        if let Some(class_id) = network.class_id() {
            if let Ok(controller) = self.controller::<NetClsController>() {
                controller.set_class(class_id as u64)?;
            }
        }

        if let Some(priorities) = network.priorities() {
            if let Ok(controller) = self.controller::<NetPrioController>() {
                for priority in priorities.iter() {
                    let eif = priority.name();
                    let prio = priority.priority() as u64;
                    controller.set_if_prio(eif, prio)?;
                }
            }
        }

        Ok(())
    }

    fn set_devices(&self, devices: &[LinuxDeviceCgroup]) -> Result<()> {
        let controller: &DevicesController = match self.controller() {
            Ok(c) => c,
            Err(_) => return Ok(()),
        };

        for device in devices.iter() {
            let devtype =
                DeviceType::from_char(device.typ().unwrap_or_default().as_str().chars().next())
                    .ok_or(Error::InvalidLinuxResource)?;

            let perm = device
                .access()
                .as_ref()
                .unwrap_or(&String::new())
                .chars()
                .filter_map(|perm| match perm {
                    'r' => Some(DevicePermissions::Read),
                    'w' => Some(DevicePermissions::Write),
                    'm' => Some(DevicePermissions::MkNod),
                    _ => None,
                })
                .collect::<Vec<_>>();

            let major = device.major().unwrap_or(0);
            let minor = device.minor().unwrap_or(0);

            if device.allow() {
                controller.allow_device(devtype, major, minor, &perm)?;
            } else {
                controller.deny_device(devtype, major, minor, &perm)?;
            }
        }

        Ok(())
    }

    /// Set the controller topdown from root in cgroup hierarchy. The `f`
    /// is going to be applied to:
    /// -> root [not included]
    ///   -> root's child
    ///     -> ...
    ///       -> self.cgroup's parent
    ///         -> self.cgroup [not included]
    ///
    /// Please see `enable_cpus_topdown()` for more details.
    ///
    /// Please note that `self.cgroup` is not included. If you really want
    /// that, you should do it manually.
    fn set_controller_topdown<T, F>(&self, f: F) -> Result<()>
    where
        for<'a> &'a T: From<&'a Subsystem>,
        T: Controller + ControllIdentifier,
        for<'a> F: Fn(&'a T) -> Result<()>,
    {
        let root = hierarchies::auto().root_control_group();
        let controller: &T = root
            .controller_of()
            .ok_or(FsError::new(FsErrorKind::SubsystemsEmpty))?;
        let root_path = Path::new(controller.path());
        let root_path_str = root_path.to_string_lossy().to_string();

        let controller: &T = self.controller()?;
        let path = Path::new(controller.path());

        // Push path's ancestors onto a stack, so the stack looks like:
        // path's parent, path's grandparent, ..., root
        let mut path_stack = vec![];
        for parent in path.ancestors() {
            if parent == root_path {
                break;
            }
            path_stack.push(parent);
        }

        // Pop from the stack
        while let Some(p) = path_stack.pop() {
            let relative_path = p
                .to_str()
                .unwrap()
                .trim_start_matches(&root_path_str)
                // Makes sure the starting slash is removed
                .trim_start_matches("/");
            let cgroup = Cgroup::new(hierarchies::auto(), relative_path)?;
            let controller: &T = cgroup
                .controller_of()
                .ok_or(FsError::new(FsErrorKind::SubsystemsEmpty))?;
            f(controller)?;
        }

        Ok(())
    }

    fn cpu_acct_stats(&self) -> Result<CpuAcctStats> {
        let controller: &CpuAcctController = self.controller()?;
        let cpu_acct = controller.cpuacct();

        let user_usage = parse_value_from_tuples(&cpu_acct.stat, "user").unwrap_or_default();

        let system_usage = parse_value_from_tuples(&cpu_acct.stat, "system").unwrap_or_default();

        let usage_percpu: Vec<u64> = cpu_acct
            .usage_percpu
            .lines()
            .filter_map(|line| line.parse::<u64>().ok())
            .collect();

        Ok(CpuAcctStats {
            user_usage,
            system_usage,
            total_usage: cpu_acct.usage,
            usage_percpu,
        })
    }

    fn cpu_throttling_stats(&self) -> Result<CpuThrottlingStats> {
        let controller: &CpuController = self.controller()?;
        let stats = controller.cpu().stat;

        let periods = parse_value_from_tuples(&stats, "nr_periods").unwrap_or_default();
        let throttled_periods = parse_value_from_tuples(&stats, "nr_throttled").unwrap_or_default();
        let throttled_time = parse_value_from_tuples(&stats, "throttled_time").unwrap_or_default();

        Ok(CpuThrottlingStats {
            periods,
            throttled_periods,
            throttled_time,
        })
    }

    fn cpu_cgroup_stats(&self) -> CpuCgroupStats {
        CpuCgroupStats {
            cpu_acct: self.cpu_acct_stats().ok(),
            cpu_throttling: self.cpu_throttling_stats().ok(),
        }
    }

    fn memory_stats(&self) -> Result<MemoryStats> {
        let controller: &MemController = self.controller()?;
        let memory_stats = controller.memory_stat();

        Ok(MemoryStats {
            usage: memory_stats.usage_in_bytes,
            max_usage: memory_stats.max_usage_in_bytes,
            limit: memory_stats.limit_in_bytes,
            fail_cnt: memory_stats.fail_cnt,
        })
    }

    fn memory_swap_stats(&self) -> Result<MemoryStats> {
        let controller: &MemController = self.controller()?;
        let memory_swap_stats = controller.memswap();

        Ok(MemoryStats {
            usage: memory_swap_stats.usage_in_bytes,
            max_usage: memory_swap_stats.max_usage_in_bytes,
            limit: memory_swap_stats.limit_in_bytes,
            fail_cnt: memory_swap_stats.fail_cnt,
        })
    }

    fn kernel_memory_stats(&self) -> Result<MemoryStats> {
        let controller: &MemController = self.controller()?;
        let kmem_stats = controller.kmem_stat();

        Ok(MemoryStats {
            usage: kmem_stats.usage_in_bytes,
            max_usage: kmem_stats.max_usage_in_bytes,
            limit: kmem_stats.limit_in_bytes,
            fail_cnt: kmem_stats.fail_cnt,
        })
    }

    fn memory_cgroup_stats(&self) -> MemoryCgroupStats {
        let memory = self.memory_stats().ok();
        let memory_swap = self.memory_swap_stats().ok();
        let kernel_memory = self.kernel_memory_stats().ok();

        let mut memory = MemoryCgroupStats {
            memory,
            memory_swap,
            kernel_memory,
            ..Default::default()
        };

        let memory_stats = self
            .controller::<MemController>()
            .map(|c| c.memory_stat())
            .ok();
        if let Some(memstats) = &memory_stats {
            memory.use_hierarchy = memstats.use_hierarchy == 1;
            // Copy items from memstats.stat
            memory.cache = memstats.stat.cache;
            memory.rss = memstats.stat.rss;
            memory.rss_huge = memstats.stat.rss_huge;
            memory.shmem = memstats.stat.shmem;
            memory.mapped_file = memstats.stat.mapped_file;
            memory.dirty = memstats.stat.dirty;
            memory.writeback = memstats.stat.writeback;
            memory.swap = memstats.stat.swap;
            memory.pgpgin = memstats.stat.pgpgin;
            memory.pgpgout = memstats.stat.pgpgout;
            memory.pgfault = memstats.stat.pgfault;
            memory.pgmajfault = memstats.stat.pgmajfault;
            memory.inactive_anon = memstats.stat.inactive_anon;
            memory.active_anon = memstats.stat.active_anon;
            memory.inactive_file = memstats.stat.inactive_file;
            memory.active_file = memstats.stat.active_file;
            memory.unevictable = memstats.stat.unevictable;
            memory.hierarchical_memory_limit = memstats.stat.hierarchical_memory_limit;
            memory.hierarchical_memsw_limit = memstats.stat.hierarchical_memsw_limit;
            memory.total_cache = memstats.stat.total_cache;
            memory.total_rss = memstats.stat.total_rss;
            memory.total_rss_huge = memstats.stat.total_rss_huge;
            memory.total_shmem = memstats.stat.total_shmem;
            memory.total_mapped_file = memstats.stat.total_mapped_file;
            memory.total_dirty = memstats.stat.total_dirty;
            memory.total_writeback = memstats.stat.total_writeback;
            memory.total_swap = memstats.stat.total_swap;
            memory.total_pgpgin = memstats.stat.total_pgpgin;
            memory.total_pgpgout = memstats.stat.total_pgpgout;
            memory.total_pgfault = memstats.stat.total_pgfault;
            memory.total_pgmajfault = memstats.stat.total_pgmajfault;
            memory.total_inactive_anon = memstats.stat.total_inactive_anon;
            memory.total_active_anon = memstats.stat.total_active_anon;
            memory.total_inactive_file = memstats.stat.total_inactive_file;
            memory.total_active_file = memstats.stat.total_active_file;
            memory.total_unevictable = memstats.stat.total_unevictable;
        }

        memory
    }

    fn pids_cgroup_stats(&self) -> PidsCgroupStats {
        let controller: &PidController = match self.controller() {
            Ok(controller) => controller,
            Err(_) => return PidsCgroupStats::default(),
        };
        let current = controller.get_pid_current().unwrap_or_default();
        let limit = controller
            .get_pid_max()
            .map(|mv| match mv {
                MaxValue::Value(limit) => limit,
                MaxValue::Max => 0,
            })
            .unwrap_or_default();

        PidsCgroupStats { current, limit }
    }

    fn blkio_stats_v1(&self) -> Result<BlkioCgroupStats> {
        let controller: &BlkIoController = self.controller()?;
        let blkio = controller.blkio();

        if blkio.io_serviced_recursive.is_empty() {
            Ok(BlkioCgroupStats {
                io_service_bytes_recursive: BlkioStat::from_io_services(
                    &blkio.throttle.io_service_bytes,
                ),
                io_serviced_recursive: BlkioStat::from_io_services(&blkio.throttle.io_serviced),
                ..Default::default()
            })
        } else {
            Ok(BlkioCgroupStats {
                io_service_bytes_recursive: BlkioStat::from_io_services(
                    &blkio.io_service_bytes_recursive,
                ),
                io_serviced_recursive: BlkioStat::from_io_services(&blkio.io_serviced_recursive),
                io_queued_recursive: BlkioStat::from_io_services(&blkio.io_queued_recursive),
                io_service_time_recursive: BlkioStat::from_io_services(
                    &blkio.io_service_time_recursive,
                ),
                io_wait_time_recursive: BlkioStat::from_io_services(&blkio.io_wait_time_recursive),
                io_merged_recursive: BlkioStat::from_io_services(&blkio.io_merged_recursive),
                io_time_recursive: BlkioStat::from_blk_io_data(&blkio.time_recursive),
                sectors_recursive: BlkioStat::from_blk_io_data(&blkio.sectors_recursive),
            })
        }
    }

    fn blkio_stats_v2(&self) -> Result<BlkioCgroupStats> {
        let controller: &BlkIoController = self.controller()?;
        let blkio = controller.blkio();

        Ok(BlkioCgroupStats {
            io_service_bytes_recursive: BlkioStat::from_io_stats(&blkio.io_stat),
            ..Default::default()
        })
    }

    fn blkio_cgroup_stats(&self) -> BlkioCgroupStats {
        if self.v2() {
            self.blkio_stats_v2()
        } else {
            self.blkio_stats_v1()
        }
        .unwrap_or_default()
    }

    fn huge_tlb_cgroup_stats(&self) -> HugeTlbCgroupStats {
        let controller: &HugeTlbController = match self.controller() {
            Ok(controller) => controller,
            Err(_) => return HugeTlbCgroupStats::default(),
        };

        let sizes = controller.get_sizes();
        sizes
            .iter()
            .map(|s| {
                let usage = controller.usage_in_bytes(s).unwrap_or_default();
                let max_usage = controller.max_usage_in_bytes(s).unwrap_or_default();
                let fail_cnt = controller.failcnt(s).unwrap_or_default();

                let stat = HugeTlbStat {
                    usage,
                    max_usage,
                    fail_cnt,
                };

                (s.to_string(), stat)
            })
            .collect()
    }

    fn devices_cgroup_stats(&self) -> DevicesCgroupStats {
        let controller: &DevicesController = match self.controller() {
            Ok(controller) => controller,
            Err(_) => return DevicesCgroupStats::default(),
        };

        let list = controller
            .allowed_devices()
            .map(|devs| {
                devs.iter()
                    .map(|dev| DeviceCgroupStat {
                        dev_type: dev.devtype.to_char().to_string(),
                        major: dev.major,
                        minor: dev.minor,
                        access: {
                            let mut access = String::new();
                            if dev.access.contains(&DevicePermissions::Read) {
                                access.push('r');
                            }
                            if dev.access.contains(&DevicePermissions::Write) {
                                access.push('w');
                            }
                            if dev.access.contains(&DevicePermissions::MkNod) {
                                access.push('m');
                            }
                            access
                        },
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        DevicesCgroupStats { list }
    }
}

impl Manager for FsManager {
    fn add_proc(&mut self, tgid: CgroupPid) -> Result<()> {
        self.create_cgroups()?;
        self.cgroup.add_task_by_tgid(tgid)?;
        Ok(())
    }

    fn add_thread(&mut self, pid: CgroupPid) -> Result<()> {
        self.create_cgroups()?;

        self.cgroup.add_task(pid).or_else(|err| {
            // Try to add_proc with the pid when threaded cgroup is
            // disabled in cgroup v2.
            if err.kind() == &FsErrorKind::CgroupMode && self.v2() {
                self.add_proc(pid)
            } else {
                Err(Error::Cgroupfs(err))
            }
        })
    }

    fn pids(&self) -> Result<Vec<CgroupPid>> {
        Ok(self
            .controller::<MemController>()
            .map_err(Error::Cgroupfs)?
            .tasks())
    }

    fn freeze(&self, state: FreezerState) -> Result<()> {
        let controller: &FreezerController = self.controller()?;

        match state {
            FreezerState::Thawed => controller.thaw()?,
            FreezerState::Frozen => controller.freeze()?,
            FreezerState::Freezing => return Err(Error::InvalidArgument),
        }

        Ok(())
    }

    fn destroy(&mut self) -> Result<()> {
        if !self.exists() {
            return Ok(());
        }

        // Before deleting the cgroup, we should move processes in the
        // cgroup to the root cgroup. Otherwise, we'll have a "Device or
        // resource busy" error.
        if self.v2() {
            for tgid in self.cgroup.procs() {
                // Ignore all errors as long as the cgroup is deleted.
                let _ = self.cgroup.remove_task_by_tgid(tgid);
            }
        } else {
            for pid in self.cgroup.tasks() {
                // Ditto.
                let _ = self.cgroup.remove_task(pid);
            }
        }

        self.cgroup.delete()?;
        Ok(())
    }

    fn set(&mut self, resources: &LinuxResources) -> Result<()> {
        if let Some(cpu) = resources.cpu() {
            self.set_cpuset(cpu)?;
            self.set_cpu(cpu)?;
        }

        if let Some(memory) = resources.memory() {
            self.set_memory(memory)?;
        }

        if let Some(pid) = resources.pids() {
            self.set_pids(pid)?;
        }

        if let Some(blkio) = resources.block_io() {
            self.set_blkio(blkio)?;
        }

        if let Some(hugepage_limits) = resources.hugepage_limits() {
            self.set_hugepages(hugepage_limits)?;
        }

        if let Some(network) = resources.network() {
            self.set_network(network)?;
        }

        if let Some(devices) = resources.devices() {
            self.set_devices(devices)?;
        }

        Ok(())
    }

    fn cgroup_path(&self, subsystem: Option<&str>) -> Result<String> {
        if self.v2() {
            return Ok(join_path(UNIFIED_MOUNTPOINT, &self.base));
        }

        let subsystem = subsystem
            .ok_or_else(|| FsError::new(FsErrorKind::InvalidPath))
            .map_err(Error::Cgroupfs)?;
        let path = self
            .paths
            .get(subsystem)
            .ok_or(FsError::new(FsErrorKind::SubsystemsEmpty))
            .map_err(Error::Cgroupfs)?;

        Ok(path.clone())
    }

    fn enable_cpus_topdown(&self, cpus: &str) -> Result<()> {
        if cpus.is_empty() {
            return Ok(());
        }

        self.set_controller_topdown(|c: &CpuSetController| {
            c.set_cpus(cpus).map_err(Error::Cgroupfs)
        })?;

        Ok(())
    }

    fn stats(&self) -> CgroupStats {
        CgroupStats {
            cpu: self.cpu_cgroup_stats(),
            memory: self.memory_cgroup_stats(),
            pids: self.pids_cgroup_stats(),
            blkio: self.blkio_cgroup_stats(),
            hugetlb: self.huge_tlb_cgroup_stats(),
            devices: self.devices_cgroup_stats(),
        }
    }

    fn paths(&self) -> &HashMap<String, String> {
        &self.paths
    }

    fn mounts(&self) -> &HashMap<String, String> {
        &self.mounts
    }

    fn serialize(&self) -> Result<String> {
        let json = serde_json::to_string(self)?;
        Ok(json)
    }

    fn systemd(&self) -> bool {
        false
    }

    fn v2(&self) -> bool {
        self.cgroup.v2()
    }
}

/// Parse cgroup subsystem paths from `/proc/self/cgroup`.
fn parse_cgroup_subsystems() -> Result<HashMap<String, String>> {
    let mut cgroup_paths = HashMap::new();
    let data = fs::read_to_string(CGROUP_PATH)
        .map_err(|err| FsError::with_cause(FsErrorKind::FsError, err))
        .map_err(Error::Cgroupfs)?;

    // Expected line format: `10:memory:/user.slice`
    for line in data.lines() {
        let parts: Vec<&str> = line.split(':').collect();
        if parts.len() != 3 {
            // Ignore corrupt lines
            continue;
        }
        let subsystems = parts[1].split(',');
        let path = parts[2];
        subsystems.for_each(|subsystem| {
            cgroup_paths.insert(subsystem.to_string(), path.to_string());
        });
    }

    Ok(cgroup_paths)
}

/// Parse cgroup mount information from `/proc/self/mountinfo`.
fn parse_cgroup_mountinfo(paths: &HashMap<String, String>) -> Result<HashMap<String, String>> {
    let mut mounts = HashMap::new();
    let data = fs::read_to_string(MOUNTINFO_PATH)
        .map_err(|err| FsError::with_cause(FsErrorKind::FsError, err))
        .map_err(Error::Cgroupfs)?;

    for line in data.lines() {
        let parts: Vec<&str> = line.splitn(2, " - ").collect();
        let part1: Vec<&str> = parts[0].split(' ').collect();
        let part2: Vec<&str> = parts[1].split(' ').collect();

        if part2.len() != 3 {
            continue;
        }

        let fs_type = part2[0];
        if fs_type != "cgroup" && fs_type != "cgroup2" {
            continue;
        }

        let super_opts: Vec<&str> = part2[2].split(',').collect();
        for opt in super_opts.iter() {
            // If opt matchs the one of cgroup subsystems
            if paths.contains_key(*opt) {
                let mountpoint = part1[4];
                mounts.insert(opt.to_string(), mountpoint.to_string());
            }
        }
    }

    Ok(mounts)
}

pub(crate) fn join_path(base: &str, path: &str) -> String {
    let base = Path::new(base);
    base.join(path).to_string_lossy().to_string()
}

/// Parse the value of an item from a tuple string split by whitespace.
///
/// For example, we have a tuple string like:
///
/// let tuple_str: &str = "system 100000\nuser 200000";
///
/// assert_eq!(
///     parse_value_from_tuples::<u64>(tuple_str, "user"),
///     Some(200000),
/// );
/// assert_eq!(
///     parse_value_from_tuples::<u64>(tuple_str, "user1"),
///     None,
/// );
fn parse_value_from_tuples<T>(tuple_str: &str, item: &str) -> Option<T>
where
    T: FromStr,
{
    tuple_str.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let current_item = parts.next()?;
        let value = parts.next()?;
        if current_item != item {
            return None;
        }
        value.parse::<T>().ok()
    })
}

impl BlkioStat {
    fn from_io_services(io_services: &[IoService]) -> Vec<Self> {
        let mut stats = Vec::new();

        for service in io_services.iter() {
            let major = service.major as u64;
            let minor = service.minor as u64;

            stats.push(BlkioStat {
                major,
                minor,
                op: "read".to_string(),
                value: service.read,
            });

            stats.push(BlkioStat {
                major,
                minor,
                op: "write".to_string(),
                value: service.write,
            });

            stats.push(BlkioStat {
                major,
                minor,
                op: "sync".to_string(),
                value: service.sync,
            });

            stats.push(BlkioStat {
                major,
                minor,
                op: "async".to_string(),
                value: service.r#async,
            });

            stats.push(BlkioStat {
                major,
                minor,
                op: "total".to_string(),
                value: service.total,
            });
        }

        stats
    }

    fn from_io_stats(io_stats: &[IoStat]) -> Vec<Self> {
        let mut stats = Vec::new();

        for stat in io_stats.iter() {
            let major = stat.major as u64;
            let minor = stat.minor as u64;

            stats.push(BlkioStat {
                major,
                minor,
                op: "read".to_string(),
                value: stat.rbytes,
            });

            stats.push(BlkioStat {
                major,
                minor,
                op: "write".to_string(),
                value: stat.wbytes,
            });

            stats.push(BlkioStat {
                major,
                minor,
                op: "rios".to_string(),
                value: stat.rios,
            });

            stats.push(BlkioStat {
                major,
                minor,
                op: "wios".to_string(),
                value: stat.wios,
            });

            stats.push(BlkioStat {
                major,
                minor,
                op: "dbytes".to_string(),
                value: stat.dbytes,
            });

            stats.push(BlkioStat {
                major,
                minor,
                op: "dios".to_string(),
                value: stat.dios,
            });
        }

        stats
    }

    fn from_blk_io_data(blkiodata: &[BlkIoData]) -> Vec<Self> {
        let op = String::new();

        blkiodata
            .iter()
            .map(|item| BlkioStat {
                major: item.major as u64,
                minor: item.minor as u64,
                op: op.clone(),
                value: item.data,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the `FsManager` implementation of the `Manager` trait.
    //!
    //! Don't run tests in parallel, use `--test-threads=1`!
    //!

    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    use oci_spec::runtime::{LinuxCpuBuilder, LinuxMemoryBuilder, LinuxResourcesBuilder};

    use crate::manager::fs::*;
    use crate::manager::tests::{MEMORY_1G, MEMORY_2G, MEMORY_512M};
    use crate::tests::spawn_sleep_inf;
    use crate::{skip_if_cgroups_v1, skip_if_cgroups_v2};

    const TEST_BASE: &str = "cgroupsrs/pod";

    impl FsManager {
        pub fn cgroup(&self) -> &Cgroup {
            &self.cgroup
        }
    }

    fn clean_cgroups(path: &str) {
        let dirs = path.split("/").fold(vec![], |mut acc, dir| {
            if let Some(last) = acc.last() {
                acc.push(format!("{}/{}", last, dir));
            } else {
                acc.push(dir.to_string());
            }
            acc
        });

        for dir in dirs.iter().rev() {
            let paths = parse_cgroup_subsystems().unwrap();
            let mounts = parse_cgroup_mountinfo(&paths).unwrap();

            if hierarchies::is_cgroup2_unified_mode() {
                let full = join_path(UNIFIED_MOUNTPOINT, dir);
                let path = Path::new(&full);
                if path.exists() {
                    // kill processes in cgroup.procs
                    let processes =
                        fs::read_to_string(path.join("cgroup.procs")).unwrap_or_default();
                    for pid in processes.lines() {
                        if let Ok(pid) = pid.parse() {
                            // kill the process
                            let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
                        }
                    }
                    fs::remove_dir(path).unwrap();
                }
            } else {
                for (subsystem, mountpoint) in mounts.iter() {
                    let full = join_path(mountpoint, paths.get(subsystem).unwrap());
                    let full = join_path(&full, dir);
                    let path = Path::new(&full);
                    if path.exists() {
                        // kill processes in the cgroup, by going through
                        // `tasks`
                        let tasks = fs::read_to_string(path.join("tasks")).unwrap_or_default();
                        for pid in tasks.lines() {
                            if let Ok(pid) = pid.parse() {
                                // kill the process
                                let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
                            }
                        }
                        fs::remove_dir(path).unwrap();
                    }
                }
            }
        }
    }

    fn new_manager() -> FsManager {
        clean_cgroups(TEST_BASE);
        FsManager::new(TEST_BASE).unwrap()
    }

    fn run_set_resources_failed(resources: LinuxResources) {
        let mut child = spawn_sleep_inf();
        let mut manager = new_manager();
        manager
            .add_proc(CgroupPid {
                pid: child.id() as u64,
            })
            .unwrap();
        assert!(manager.set(&resources).is_err());
        manager.destroy().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }

    fn run_set_resources<F>(linux_resources: LinuxResources, test_fn: F)
    where
        F: FnOnce(&mut FsManager),
    {
        let mut child = spawn_sleep_inf();
        let mut manager = new_manager();
        manager
            .add_proc(CgroupPid {
                pid: child.id() as u64,
            })
            .unwrap();
        manager.set(&linux_resources).unwrap();
        test_fn(&mut manager);
        manager.destroy().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn test_parse_value_from_tuples() {
        let tuple_str = "system 100000\nuser 200000";
        assert_eq!(
            parse_value_from_tuples::<u64>(tuple_str, "user"),
            Some(200000)
        );
        assert_eq!(
            parse_value_from_tuples::<u64>(tuple_str, "system"),
            Some(100000)
        );
        assert_eq!(parse_value_from_tuples::<u64>(tuple_str, "user1"), None);
    }

    #[test]
    fn test_paths_and_mounts() {
        let mut manager = new_manager();

        for (subsystem, mountpoint) in manager.mounts() {
            let subsys = if subsystem.is_empty() {
                assert!(manager.v2());
                None
            } else {
                Some(subsystem.as_str())
            };
            let path = manager.cgroup_path(subsys).unwrap();
            let path = join_path(mountpoint, &path);
            assert!(Path::new(&path).exists(), "Cgroup {} does not exist", path);
        }

        manager.destroy().unwrap();
    }

    #[test]
    fn test_destroy() {
        let mut manager = new_manager();
        manager.create_cgroups().unwrap();

        let cgroup_path = if manager.v2() {
            manager.cgroup_path(None).unwrap()
        } else {
            manager.cgroup_path(Some("memory")).unwrap()
        };
        assert!(
            Path::new(&cgroup_path).exists(),
            "Cgroup should exist before destroy"
        );

        manager.destroy().unwrap();
        assert!(
            !Path::new(&cgroup_path).exists(),
            "Cgroup should not exist after destroy"
        );
    }

    #[test]
    fn test_set_cpu() {
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
            let controller: &CpuController = manager.controller().unwrap();
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
        skip_if_cgroups_v1!();

        // expected failure: swap < limit
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
            let controller: &MemController = manager.controller().unwrap();
            let memory_stat = controller.memory_stat();
            let memory_swap_stat = controller.memswap();

            assert_eq!(memory_stat.limit_in_bytes, MEMORY_512M);
            assert_eq!(memory_swap_stat.limit_in_bytes, MEMORY_512M);
            assert_eq!(memory_stat.soft_limit_in_bytes, MEMORY_2G);
        });
    }

    #[test]
    fn test_set_memory_v1() {
        skip_if_cgroups_v2!();

        let linux_memory = LinuxMemoryBuilder::default()
            .limit(MEMORY_512M)
            .swap(MEMORY_512M)
            .reservation(MEMORY_512M)
            .disable_oom_killer(true)
            .swappiness(50u64)
            .build()
            .unwrap();
        let linux_resources = LinuxResourcesBuilder::default()
            .memory(linux_memory)
            .build()
            .unwrap();
        run_set_resources(linux_resources, |manager| {
            let controller: &MemController = manager.controller().unwrap();
            let memory_stat = controller.memory_stat();
            let memory_swap_stat = controller.memswap();

            assert_eq!(memory_stat.limit_in_bytes, MEMORY_512M);
            assert_eq!(memory_swap_stat.limit_in_bytes, MEMORY_512M);
            assert_eq!(memory_stat.soft_limit_in_bytes, MEMORY_512M);
            assert_eq!(memory_stat.swappiness, 50);
            assert!(memory_stat.oom_control.oom_kill_disable);
        });

        // expected failure: swapiness too high
        let linux_memory = LinuxMemoryBuilder::default()
            .swappiness(101u64)
            .build()
            .unwrap();
        let linux_resources = LinuxResourcesBuilder::default()
            .memory(linux_memory)
            .build()
            .unwrap();
        run_set_resources_failed(linux_resources);
    }

    fn parse_cpu_list(online_str: &str) -> Vec<u32> {
        let mut cpus = Vec::new();
        for part in online_str.trim().split(',') {
            if let Some((start, end)) = part.split_once('-') {
                let start: u32 = start.parse().unwrap();
                let end: u32 = end.parse().unwrap();
                cpus.extend(start..=end);
            } else {
                cpus.push(part.parse().unwrap());
            }
        }
        cpus
    }

    #[test]
    fn test_enable_cpus_topdown() {
        let cpuset_cpus_path = format!("/sys/fs/cgroup/{}/cpuset.cpus", TEST_BASE);
        let online_cpus = fs::read_to_string("/sys/devices/system/cpu/online").unwrap();
        let cpus = parse_cpu_list(&online_cpus);

        // Skip this test if there are less than 2 CPUs online
        if cpus.len() < 2 {
            return;
        }

        let linux_cpu = LinuxCpuBuilder::default()
            .cpus(format!("{}", cpus[0]))
            .build()
            .unwrap();
        let linux_resources = LinuxResourcesBuilder::default()
            .cpu(linux_cpu)
            .build()
            .unwrap();
        run_set_resources(linux_resources, |manager| {
            let cpus1 = fs::read_to_string(&cpuset_cpus_path).unwrap();
            let cpus1 = parse_cpu_list(&cpus1);
            assert_eq!(cpus[..1], cpus1);

            manager
                .enable_cpus_topdown(&format!("{},{}", cpus[0], cpus[1]))
                .unwrap();
            let cpuset_cpus = fs::read_to_string(&cpuset_cpus_path).unwrap();
            let cpus2 = parse_cpu_list(&cpuset_cpus);
            assert_eq!(cpus[..2], cpus2);
        });
    }

    #[test]
    fn test_systemd() {
        let mut manager = new_manager();
        assert!(!manager.systemd(), "FsManager should not be systemd");
        manager.destroy().unwrap();
    }
}
