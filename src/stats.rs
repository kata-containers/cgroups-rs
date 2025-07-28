// Copyright (c) 2018 Levente Kurusa
// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct CgroupStats {
    pub cpu: CpuCgroupStats,
    pub memory: MemoryCgroupStats,
    pub pids: PidsCgroupStats,
    pub blkio: BlkioCgroupStats,
    pub hugetlb: HugeTlbCgroupStats,
}

#[derive(Debug, Default)]
pub struct CpuCgroupStats {
    pub cpu_acct: Option<CpuAcctStats>,
    pub cpu_throttling: Option<CpuThrottlingStats>,
}

#[derive(Debug, Default)]
pub struct CpuAcctStats {
    /// Usage in userspace, read from `cpuacct.stat` from the line starting
    /// with `user`. Set 0 if no data.
    pub user_usage: u64,
    /// Usage in kernelspace, read from `cpuacct.stat` from the line
    /// starting with `system`. Set 0 if no data.
    pub system_usage: u64,
    /// Total usage, read from `cpuacct.usage`. Set 0 if no data.
    pub total_usage: u64,
    /// Per-CPU usage, read from `cpuacct.usage_percpu`.
    pub usage_percpu: Vec<u64>,
}

#[derive(Debug, Default)]
pub struct CpuThrottlingStats {
    /// Periods, read from `cpu.stat` from the line starting with
    /// `nr_periods`. Set 0 if no data.
    pub periods: u64,
    /// Throttled periods, read from `cpu.stat` from the line starting with
    /// `nr_throttled`. Set 0 if no data.
    pub throttled_periods: u64,
    /// Throttled time, read from `cpu.stat` from the line starting with
    /// `throttled_time`. Set 0 if no data.
    pub throttled_time: u64,
}

#[derive(Debug, Default)]
pub struct MemoryCgroupStats {
    pub memory: Option<MemoryStats>,
    pub memory_swap: Option<MemoryStats>,
    pub kernel_memory: Option<MemoryStats>,

    /// Use hierarchy, read from `memory.use_hierarchy` in cgroups v1. Only
    /// available in cgroups v1.
    pub use_hierarchy: bool,

    // The following data is read from `memory.stat`, see also
    // `crate::fs::memory::MemoryStat::stat`.
    pub cache: u64,
    pub rss: u64,
    pub rss_huge: u64,
    pub shmem: u64,
    pub mapped_file: u64,
    pub dirty: u64,
    pub writeback: u64,
    pub swap: u64,
    pub pgpgin: u64,
    pub pgpgout: u64,
    pub pgfault: u64,
    pub pgmajfault: u64,
    pub inactive_anon: u64,
    pub active_anon: u64,
    pub inactive_file: u64,
    pub active_file: u64,
    pub unevictable: u64,
    pub hierarchical_memory_limit: i64,
    pub hierarchical_memsw_limit: i64,
    pub total_cache: u64,
    pub total_rss: u64,
    pub total_rss_huge: u64,
    pub total_shmem: u64,
    pub total_mapped_file: u64,
    pub total_dirty: u64,
    pub total_writeback: u64,
    pub total_swap: u64,
    pub total_pgpgin: u64,
    pub total_pgpgout: u64,
    pub total_pgfault: u64,
    pub total_pgmajfault: u64,
    pub total_inactive_anon: u64,
    pub total_active_anon: u64,
    pub total_inactive_file: u64,
    pub total_active_file: u64,
    pub total_unevictable: u64,
}

#[derive(Debug, Default)]
pub struct MemoryStats {
    /// Memory [swap] usage, read from `memory[.memsw].usage_in_bytes` in
    /// cgroups v1 and `memory[.swap].current` in cgroups v2.
    pub usage: u64,
    /// Maximum memory [swap] usage observed by cgroups, read from
    /// `memory[.memsw].max_usage_in_bytes` in cgroups v1 and
    /// `memory[.swap].peak` in cgroups v2.
    pub max_usage: u64,
    /// Memory [swap] limit, read from `memory[.memsw].limit_in_bytes` in
    /// cgroups v1 and `memory[.swap].max` in cgroups v2.
    pub limit: i64,
    /// Failure count, read from `memory[.memsw].failcnt`. Only available in
    /// cgroups v1.
    pub fail_cnt: u64,
}

#[derive(Debug, Default)]
pub struct PidsCgroupStats {
    /// Current number of processes in the cgroup, read from `pids.current`.
    pub current: u64,
    /// Maximum number of processes in the cgroup, read from `pids.limit`.
    pub limit: i64,
}

#[derive(Debug, Default)]
pub struct BlkioCgroupStats {
    pub io_service_bytes_recursive: Vec<BlkioStat>,
    pub io_serviced_recursive: Vec<BlkioStat>,
    pub io_queued_recursive: Vec<BlkioStat>,
    pub io_service_time_recursive: Vec<BlkioStat>,
    pub io_wait_time_recursive: Vec<BlkioStat>,
    pub io_merged_recursive: Vec<BlkioStat>,
    pub io_time_recursive: Vec<BlkioStat>,
    pub sectors_recursive: Vec<BlkioStat>,
}

#[derive(Debug, Default)]
pub struct BlkioStat {
    pub major: u64,
    pub minor: u64,
    pub op: String,
    pub value: u64,
}

/// A structure representing the statistics of the `hugetlb` subsystem of a
/// Cgroup. The key is the huge page size, and the value is the statistics
/// for that size.
pub type HugeTlbCgroupStats = HashMap<String, HugeTlbStat>;

#[derive(Debug, Default)]
pub struct HugeTlbStat {
    pub usage: u64,
    pub max_usage: u64,
    pub fail_cnt: u64,
}
