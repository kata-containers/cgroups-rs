// Copyright (c) 2018 Levente Kurusa
// Copyright (c) 2020 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

//! This module contains the implementation of the `freezer` cgroup subsystem.
//!
//! See the Kernel's documentation for more information about this subsystem, found at:
//!  [Documentation/cgroup-v1/freezer-subsystem.txt](https://www.kernel.org/doc/Documentation/cgroup-v1/freezer-subsystem.txt)
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;

use crate::fs::error::ErrorKind::*;
use crate::fs::error::*;
use crate::fs::{
    read_keyed_u64, read_u64_from, ControllIdentifier, ControllerInternal, Controllers, Resources,
    Subsystem,
};
use crate::FreezerState;

/// A controller that allows controlling the `freezer` subsystem of a Cgroup.
///
/// In essence, this subsystem allows the user to freeze and thaw (== "un-freeze") the processes in
/// the control group. This is done _transparently_ so that neither the parent, nor the children of
/// the processes can observe the freeze.
///
/// Note that if the control group is currently in the `Frozen` or `Freezing` state, then no
/// processes can be added to it.
#[derive(Debug, Clone)]
pub struct FreezerController {
    base: PathBuf,
    path: PathBuf,
    v2: bool,
}

impl ControllerInternal for FreezerController {
    fn control_type(&self) -> Controllers {
        Controllers::Freezer
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

    fn apply(&self, _res: &Resources) -> Result<()> {
        Ok(())
    }
}

impl ControllIdentifier for FreezerController {
    fn controller_type() -> Controllers {
        Controllers::Freezer
    }
}

impl<'a> From<&'a Subsystem> for &'a FreezerController {
    fn from(sub: &'a Subsystem) -> &'a FreezerController {
        unsafe {
            match sub {
                Subsystem::Freezer(c) => c,
                _ => {
                    assert_eq!(1, 0);
                    let v = std::mem::MaybeUninit::uninit();
                    v.assume_init()
                }
            }
        }
    }
}

impl FreezerController {
    /// Contructs a new `FreezerController` with `root` serving as the root of the control group.
    pub fn new(point: PathBuf, root: PathBuf, v2: bool) -> Self {
        Self {
            base: root,
            path: point,
            v2,
        }
    }
    /// Freezes the processes in the control group.
    pub fn freeze(&self) -> Result<()> {
        let mut file_name = "freezer.state";
        let mut content = "FROZEN".to_string();
        if self.v2 {
            file_name = "cgroup.freeze";
            content = "1".to_string();
        }

        self.open_path(file_name, true).and_then(|mut file| {
            file.write_all(content.as_ref())
                .map_err(|e| Error::with_cause(WriteFailed(file_name.to_string(), content), e))
        })
    }

    /// Thaws, that is, unfreezes the processes in the control group.
    pub fn thaw(&self) -> Result<()> {
        let mut file_name = "freezer.state";
        let mut content = "THAWED".to_string();
        if self.v2 {
            file_name = "cgroup.freeze";
            content = "0".to_string();
        }
        self.open_path(file_name, true).and_then(|mut file| {
            file.write_all(content.as_ref())
                .map_err(|e| Error::with_cause(WriteFailed(file_name.to_string(), content), e))
        })
    }

    fn read_v1_state(&self, file_name: &str) -> Result<FreezerState> {
        self.open_path(file_name, false).and_then(|mut file| {
            let mut s = String::new();
            let res = file.read_to_string(&mut s);
            match res {
                Ok(_) => match s.trim() {
                    "FROZEN" => Ok(FreezerState::Frozen),
                    "THAWED" => Ok(FreezerState::Thawed),
                    "1" => Ok(FreezerState::Frozen),
                    "0" => Ok(FreezerState::Thawed),
                    "FREEZING" => Ok(FreezerState::Freezing),
                    _ => Err(Error::new(ParseError)),
                },
                Err(e) => Err(Error::with_cause(ReadFailed(file_name.to_string()), e)),
            }
        })
    }

    fn read_frozen_counter(&self, file_name: &str) -> Result<u64> {
        self.open_path(file_name, false).and_then(|file| {
            let reader = BufReader::new(file);
            read_keyed_u64(reader, "frozen", file_name)?
                .ok_or_else(|| Error::new(ErrorKind::ParseError))
        })
    }

    /// Retrieve the state of processes in the control group.
    ///
    /// On cgroup v2, this reflects the kernel's actual state by combining
    /// `cgroup.freeze` (the requested state) with `cgroup.events` (the
    /// completed state). As a result, it may transiently return
    /// [`FreezerState::Freezing`] immediately after [`freeze`](Self::freeze)
    /// (before the kernel finishes freezing) or
    /// [`FreezerState::Frozen`] immediately after [`thaw`](Self::thaw)
    /// (before the kernel updates `cgroup.events`). Poll `state()` if you
    /// need to observe the final state.
    pub fn state(&self) -> Result<FreezerState> {
        if !self.v2 {
            return self.read_v1_state("freezer.state");
        }

        let requested = self
            .open_path("cgroup.freeze", false)
            .and_then(read_u64_from)?;
        let completed = self.read_frozen_counter("cgroup.events")?;
        match (requested, completed) {
            (1, 1) | (0, 1) => Ok(FreezerState::Frozen),
            (1, 0) => Ok(FreezerState::Freezing),
            (0, 0) => Ok(FreezerState::Thawed),
            _ => Err(Error::new(ErrorKind::ParseError)),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cgroups-rs-freezer-test-{}-{}",
            name,
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_v1_state() {
        let dir = temp_dir("v1-state");
        let c = FreezerController::new(dir.clone(), dir.clone(), false);
        assert!(!c.is_v2());

        std::fs::write(dir.join("freezer.state"), "THAWED").unwrap();
        assert_eq!(c.state().unwrap(), FreezerState::Thawed);

        std::fs::write(dir.join("freezer.state"), "FROZEN").unwrap();
        assert_eq!(c.state().unwrap(), FreezerState::Frozen);

        std::fs::write(dir.join("freezer.state"), "FREEZING").unwrap();
        assert_eq!(c.state().unwrap(), FreezerState::Freezing);

        std::fs::write(dir.join("freezer.state"), "BOGUS").unwrap();
        assert_eq!(c.state().unwrap_err().kind(), &ErrorKind::ParseError);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_v1_freeze_thaw() {
        let dir = temp_dir("v1-freeze-thaw");
        let c = FreezerController::new(dir.clone(), dir.clone(), false);

        c.freeze().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("freezer.state")).unwrap(),
            "FROZEN"
        );

        c.thaw().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("freezer.state")).unwrap(),
            "THAWED"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_v2_state() {
        let dir = temp_dir("v2-state");
        let c = FreezerController::new(dir.clone(), dir.clone(), true);
        assert!(c.is_v2());

        std::fs::write(dir.join("cgroup.freeze"), "0").unwrap();
        std::fs::write(dir.join("cgroup.events"), "frozen 0").unwrap();
        assert_eq!(c.state().unwrap(), FreezerState::Thawed);

        std::fs::write(dir.join("cgroup.freeze"), "1").unwrap();
        assert_eq!(c.state().unwrap(), FreezerState::Freezing);

        std::fs::write(dir.join("cgroup.events"), "frozen 1").unwrap();
        assert_eq!(c.state().unwrap(), FreezerState::Frozen);

        std::fs::write(dir.join("cgroup.freeze"), "2").unwrap();
        assert_eq!(c.state().unwrap_err().kind(), &ErrorKind::ParseError);

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_v2_freeze_thaw() {
        let dir = temp_dir("v2-freeze-thaw");
        let c = FreezerController::new(dir.clone(), dir.clone(), true);

        c.freeze().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("cgroup.freeze")).unwrap(),
            "1"
        );

        c.thaw().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("cgroup.freeze")).unwrap(),
            "0"
        );

        std::fs::remove_dir_all(dir).unwrap();
    }
}
