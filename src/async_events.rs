// Copyright (c) 2020 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use futures_core::Stream;
use std::os::unix::prelude::AsRawFd;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncReadExt;

use crate::error::ErrorKind::*;
use crate::error::*;

// notify_on_oom returns channel on which you can expect event about OOM,
// if process died without OOM this channel will be closed.
pub async fn notify_on_oom_v2(key: &str, dir: &Path) -> Result<impl Stream<Item = String>> {
    register_memory_event(key, dir, "memory.oom_control", "").await
}

// notify_on_oom returns channel on which you can expect event about OOM,
// if process died without OOM this channel will be closed.
pub async fn notify_on_oom_v1(key: &str, dir: &Path) -> Result<impl Stream<Item = String>> {
    register_memory_event(key, dir, "memory.oom_control", "").await
}

// level is one of "low", "medium", or "critical"
pub async fn notify_memory_pressure(
    key: &str,
    dir: &Path,
    level: &str,
) -> Result<impl Stream<Item = String>> {
    if level != "low" && level != "medium" && level != "critical" {
        return Err(Error::from_string(format!(
            "invalid pressure level {}",
            level
        )));
    }

    register_memory_event(key, dir, "memory.pressure_level", level).await
}

async fn register_memory_event(
    key: &str,
    cg_dir: &Path,
    event_name: &str,
    arg: &str,
) -> Result<impl Stream<Item = String>> {
    let path = cg_dir.join(event_name);
    let event_file = fs::File::open(path)
        .await
        .map_err(|e| Error::with_cause(ReadFailed, e))?;

    let mut eventfd =
        tokio_eventfd::EventFd::new(0, false).map_err(|e| Error::with_cause(ReadFailed, e))?;

    let event_control_path = cg_dir.join("cgroup.event_control");
    let data;
    if arg.is_empty() {
        data = format!("{} {}", eventfd.as_raw_fd(), event_file.as_raw_fd());
    } else {
        data = format!("{} {} {}", eventfd.as_raw_fd(), event_file.as_raw_fd(), arg);
    }

    // write to file and set mode to 0700(FIXME)
    fs::write(&event_control_path, data)
        .await
        .map_err(|e| Error::with_cause(WriteFailed, e))?;

    let key = key.to_string();
    let s = async_stream::stream! {
        loop {
            let mut buf = [0; 8];
            if eventfd.read(&mut buf).await.is_err() {
                return;
            }

            // When a cgroup is destroyed, an event is sent to eventfd.
            // So if the control path is gone, return instead of notifying.
            if !Path::new(&event_control_path).exists() {
                return;
            }
            yield key.clone()
        }
    };

    Ok(s)
}
