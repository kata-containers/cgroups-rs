use std::fs::write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process;
use std::process::Command;

use crate::Cgroup;

/// Extensions to default `std::process::Command` capabilities.
pub trait CgroupsCommandExt {
    /// Sets cgroups for the process to be put into before execution of that process starts.
    ///
    /// # Examples
    /// ```
    /// use std::process::Command;
    /// use cgroups_rs::{Cgroup, CgroupPid};
    /// use cgroups_rs::cgroup_builder::CgroupBuilder;
    /// use cgroups_rs::MaxValue::Value;
    /// use cgroups_rs::process_ext::CgroupsCommandExt;
    /// use std::cell::Ref;
    ///
    /// let mut process = Command::new("ls");
    ///
    /// let hierarchy = cgroups_rs::hierarchies::auto();
    /// let cgroup_name = "test";
    /// let cg: Cgroup = CgroupBuilder::new(cgroup_name)
    ///         .cpu()
    ///         .cpus("1".to_owned())
    ///         .shares(100)
    ///         .done()
    ///         .build(hierarchy);
    /// let mut child = process.cgroups(&[&cg])
    ///         .spawn()
    ///         .expect("The 'ls' process did not spawn.");
    /// let child_pid = CgroupPid::from(&child);
    /// println!("{:?}", child.wait_with_output().expect("Expected 'ls' to provide an output'"));
    /// cg.remove_task(child_pid);
    /// cg.delete();
    ///
    /// ```
    fn cgroups(&mut self, cgroups: &[&Cgroup]) -> &mut Self;
}

impl CgroupsCommandExt for Command {
    ///Adds PID of `self` to given cgroups before the process is started using unix-specific
    /// `pre_exec` functionality. First, the PID is written into
    /// Inspired by the `cgroups-fs` crate.
    fn cgroups(&mut self, cgroups: &[&Cgroup]) -> &mut Self {
        let tasks_paths = cgroups
            .iter()
            .map(|cgroup| PathBuf::from(cgroup.path()))
            .collect::<Vec<PathBuf>>();
        unsafe {
            self.pre_exec(move || {
                let pid = process::id().to_string();
                for tasks_path in &tasks_paths {
                    write(tasks_path, &pid)?;
                }
                Ok(())
            })
        }
    }
}
