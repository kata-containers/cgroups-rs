// SPDX-License-Identifier: Apache-2.0 or MIT
//

//! This module contains the implementation of the `misc` cgroup subsystem.
//!
//! See the Kernel's documentation for more information about this subsystem, found at:
//!  [Documentation/admin-guide/cgroup-v2.rst](https://docs.kernel.org/admin-guide/cgroup-v2.html#misc)

use std::io::Write;
use std::path::PathBuf;

use crate::fs::error::*;
use crate::fs::read_string_from;
use crate::fs::{
    ControllIdentifier, ControllerInternal, Controllers, MiscMaxValue, MiscResources, Resources,
    Subsystem,
};

/// A controller that allows controlling the `misc` subsystem of a Cgroup.
///
/// In essence, using this controller one can limit and track miscellaneous
/// scalar resources that cannot be abstracted like the other cgroup resources
/// (e.g. `sev`, `sev_es`, `tdx`).
///
/// The misc controller is available on both cgroup v1 and v2 (kernel >= 5.13,
/// `CONFIG_CGROUP_MISC`); the interface files are identical on both.
#[derive(Debug, Clone)]
pub struct MiscController {
    base: PathBuf,
    path: PathBuf,
    // Records the hierarchy type; no behavior branches on it, as the misc
    // interface files are identical on cgroup v1 and v2.
    v2: bool,
}

impl ControllerInternal for MiscController {
    fn control_type(&self) -> Controllers {
        Controllers::Misc
    }
    fn get_path(&self) -> &PathBuf {
        &self.path
    }
    fn get_path_mut(&mut self) -> &mut PathBuf {
        &mut self.path
    }
    fn get_base(&self) -> &PathBuf {
        &self.base
    }

    fn is_v2(&self) -> bool {
        self.v2
    }

    fn apply(&self, res: &Resources) -> Result<()> {
        // get the resources that apply to this controller
        let res: &MiscResources = &res.misc;
        for (name, val) in &res.maximum {
            self.set_max(name, *val)?;
        }

        Ok(())
    }
}

impl ControllIdentifier for MiscController {
    fn controller_type() -> Controllers {
        Controllers::Misc
    }
}

impl<'a> From<&'a Subsystem> for &'a MiscController {
    fn from(sub: &'a Subsystem) -> &'a MiscController {
        unsafe {
            match sub {
                Subsystem::Misc(c) => c,
                _ => {
                    assert_eq!(1, 0);
                    let v = std::mem::MaybeUninit::uninit();
                    v.assume_init()
                }
            }
        }
    }
}

impl MiscController {
    /// Constructs a new `MiscController` with `root` serving as the root of the control group.
    ///
    /// Note that the `v2` flag is only informational: the misc interface files
    /// are identical on cgroup v1 and v2, so no behavior depends on it.
    pub fn new(point: PathBuf, root: PathBuf, v2: bool) -> Self {
        Self {
            base: root,
            path: point,
            v2,
        }
    }

    /// Returns the available quantity of each misc resource on the host.
    ///
    /// Note: `misc.capacity` is only shown in the root cgroup; reading it from
    /// a non-root cgroup fails.
    pub fn capacity(&self) -> Result<String> {
        self.open_path("misc.capacity", false)
            .and_then(read_string_from)
    }

    /// Returns the current usage of each misc resource in the cgroup and its children.
    pub fn current(&self) -> Result<String> {
        self.open_path("misc.current", false)
            .and_then(read_string_from)
    }

    /// Returns the historical maximum (peak) usage of each misc resource in the
    /// cgroup and its children.
    ///
    /// Note: `misc.peak` was added in Linux 6.11; reading it on older kernels fails.
    pub fn peak(&self) -> Result<String> {
        self.open_path("misc.peak", false)
            .and_then(read_string_from)
    }

    /// Returns the maximum allowed usage of each misc resource in the cgroup
    /// and its children.
    ///
    /// Note: `misc.max` is only shown in non-root cgroups.
    pub fn max(&self) -> Result<String> {
        self.open_path("misc.max", false).and_then(read_string_from)
    }

    /// Sets the maximum allowed usage of a misc resource.
    ///
    /// `resource` is the resource name, and `max` is either a numeric limit or
    /// [`MiscMaxValue::Max`] (no limit). The limit may be set higher than the
    /// resource's capacity.
    ///
    /// Note: `misc.max` only exists in non-root cgroups.
    pub fn set_max(&self, resource: &str, max: MiscMaxValue) -> Result<()> {
        let line = format!("{} {}", resource, max);
        self.open_path("misc.max", true).and_then(|mut file| {
            file.write_all(line.as_ref()).map_err(|e| {
                Error::with_cause(ErrorKind::WriteFailed("misc.max".to_string(), line), e)
            })
        })
    }

    /// Returns the number of times each resource's usage was about to go over
    /// its max boundary. The counts are hierarchical, i.e. they include the
    /// events of all descendant cgroups.
    ///
    /// The output is one `<resource>.max <count>` line per resource.
    ///
    /// Note: `misc.events` was added in Linux 5.16 and is only shown in
    /// non-root cgroups.
    pub fn events(&self) -> Result<String> {
        self.open_path("misc.events", false)
            .and_then(read_string_from)
    }

    /// Like [`MiscController::events`], but only counting events local to this
    /// cgroup itself.
    ///
    /// Note: `misc.events.local` was added in Linux 6.11 and is only shown in
    /// non-root cgroups.
    pub fn events_local(&self) -> Result<String> {
        self.open_path("misc.events.local", false)
            .and_then(read_string_from)
    }
}
