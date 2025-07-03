// Copyright (c) 2025 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use zbus::zvariant::Value as ZbusValue;

use crate::fs::hierarchies;
use crate::systemd::utils::is_slice_unit;
use crate::systemd::{
    BLOCK_IO_ACCOUNTING, CPU_ACCOUNTING, DEFAULT_DEPENDENCIES, DEFAULT_DESCRIPTION, DELEGATE,
    DESCRIPTION, IO_ACCOUNTING, MEMORY_ACCOUNTING, PIDS, SLICE, TASKS_ACCOUNTING,
    TIMEOUT_STOP_USEC, WANTS,
};

pub type Property<'a> = (&'a str, ZbusValue<'a>);

#[derive(Debug, Clone, Default)]
pub struct PropertiesBuilder {
    cpu_accounting: Option<bool>,
    // MemoryAccount is for cgroup v2 as documented in dbus. However,
    // "github.com/opencontainer/runc" uses it for all. Shall we follow the
    // same way?
    memory_accounting: Option<bool>,
    task_accounting: Option<bool>,
    // Use IO_ACCOUNTING for cgroup v2 and BLOCK_IO_ACCOUNTING for cgroup v1.
    io_accounting: Option<bool>,
    default_dependencies: Option<bool>,
    description: Option<String>,
    wants: Option<String>,
    slice: Option<String>,
    delegate: Option<bool>,
    pids: Option<Vec<u32>>,
    timeout_stop_usec: Option<u64>,
}

impl PropertiesBuilder {
    pub fn default_cgroup(slice: &str, unit: &str) -> Self {
        let mut builder = Self::default()
            .cpu_accounting(true)
            .memory_accounting(true)
            .task_accounting(true)
            .io_accounting(true)
            .default_dependencies(false)
            .description(format!("{} {}:{}", DEFAULT_DESCRIPTION, slice, unit));

        if is_slice_unit(unit) {
            // If we create a slice, the parent is defined via a Wants=.
            builder = builder.wants(slice.to_string());
        } else {
            // Otherwise it's a scope, which we put into a Slice=.
            builder = builder.slice(slice.to_string());
            // Assume scopes always support delegation (supported since systemd v218).
            builder = builder.delegate(true);
        }

        builder
    }

    pub fn cpu_accounting(mut self, enabled: bool) -> Self {
        self.cpu_accounting = Some(enabled);
        self
    }

    pub fn memory_accounting(mut self, enabled: bool) -> Self {
        self.memory_accounting = Some(enabled);
        self
    }

    pub fn task_accounting(mut self, enabled: bool) -> Self {
        self.task_accounting = Some(enabled);
        self
    }

    pub fn io_accounting(mut self, enabled: bool) -> Self {
        self.io_accounting = Some(enabled);
        self
    }

    pub fn default_dependencies(mut self, enabled: bool) -> Self {
        self.default_dependencies = Some(enabled);
        self
    }

    pub fn description(mut self, desc: String) -> Self {
        self.description = Some(desc);
        self
    }

    pub fn wants(mut self, wants: String) -> Self {
        self.wants = Some(wants);
        self
    }

    pub fn slice(mut self, slice: String) -> Self {
        self.slice = Some(slice);
        self
    }

    pub fn delegate(mut self, enabled: bool) -> Self {
        self.delegate = Some(enabled);
        self
    }

    pub fn pids(mut self, pids: Vec<u32>) -> Self {
        self.pids = Some(pids);
        self
    }

    pub fn timeout_stop_usec(mut self, timeout: u64) -> Self {
        self.timeout_stop_usec = Some(timeout);
        self
    }

    pub fn build(self) -> Vec<Property<'static>> {
        let mut props = vec![];

        if let Some(cpu_accounting) = self.cpu_accounting {
            props.push((CPU_ACCOUNTING, ZbusValue::Bool(cpu_accounting)));
        }

        if let Some(memory_accounting) = self.memory_accounting {
            props.push((MEMORY_ACCOUNTING, ZbusValue::Bool(memory_accounting)));
        }

        if let Some(task_accounting) = self.task_accounting {
            props.push((TASKS_ACCOUNTING, ZbusValue::Bool(task_accounting)));
        }

        if let Some(io_accounting) = self.io_accounting {
            if hierarchies::is_cgroup2_unified_mode() {
                props.push((IO_ACCOUNTING, ZbusValue::Bool(io_accounting)));
            } else {
                props.push((BLOCK_IO_ACCOUNTING, ZbusValue::Bool(io_accounting)));
            }
        }

        if let Some(default_dependencies) = self.default_dependencies {
            props.push((DEFAULT_DEPENDENCIES, ZbusValue::Bool(default_dependencies)));
        }

        if let Some(description) = self.description {
            props.push((DESCRIPTION, ZbusValue::Str(description.into())));
        } else {
            props.push((DESCRIPTION, ZbusValue::Str(DEFAULT_DESCRIPTION.into())));
        }

        if let Some(wants) = self.wants {
            props.push((WANTS, ZbusValue::Str(wants.into())));
        }

        if let Some(slice) = self.slice {
            props.push((SLICE, ZbusValue::Str(slice.into())));
        }

        if let Some(delegate) = self.delegate {
            props.push((DELEGATE, ZbusValue::Bool(delegate)));
        }

        if let Some(pids) = self.pids {
            props.push((PIDS, ZbusValue::Array(pids.into())));
        }

        if let Some(timeout) = self.timeout_stop_usec {
            props.push((TIMEOUT_STOP_USEC, ZbusValue::U64(timeout)));
        }

        props
    }
}
