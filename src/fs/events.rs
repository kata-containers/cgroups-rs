// Copyright (c) 2020 Ant Group
//
// SPDX-License-Identifier: Apache-2.0 or MIT
//

use nix::errno::Errno;
use nix::sys::eventfd::{EfdFlags, EventFd};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use std::fs::{self, File};
use std::io::BufReader;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::fs::error::ErrorKind::*;
use crate::fs::error::*;

fn read_oom_count(path: &Path) -> Result<u64> {
    let file = File::open(path)
        .map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;
    let reader = BufReader::new(file);
    let name = path.display().to_string();
    crate::fs::read_keyed_u64(reader, "oom", &name)?
        .ok_or_else(|| Error::from_string("oom not found".to_string()))
}

// notify_on_oom returns channel on which you can expect event about OOM,
// if cgroup was destroyed without OOM this channel will be closed.
pub fn notify_on_oom_v2(key: &str, dir: &Path) -> Result<Receiver<String>> {
    let path = dir.join("memory.events");
    let parent = dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let cgroup_name = dir.file_name().ok_or_else(|| {
        Error::from_string(format!("cannot watch cgroup removal for {}", dir.display()))
    })?;
    let inotify = Inotify::init(InitFlags::IN_CLOEXEC)
        .map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;
    let parent_watch = inotify
        .add_watch(
            parent,
            AddWatchFlags::IN_DELETE | AddWatchFlags::IN_MOVED_FROM,
        )
        .map_err(|e| Error::with_cause(ReadFailed(parent.display().to_string()), e))?;
    let events_watch = inotify
        .add_watch(&path, AddWatchFlags::IN_MODIFY)
        .map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;

    // Establish both watches before taking the baseline so changes after the
    // baseline read and removal during registration have queued events.
    let mut base = read_oom_count(&path)?;
    let (sender, receiver) = mpsc::channel();
    let key = key.to_string();
    let cgroup_name = cgroup_name.to_os_string();
    // Own the path so the watcher thread can re-check liveness after a queue
    // overflow without borrowing from the caller ('static required by spawn).
    let dir = dir.to_path_buf();

    thread::spawn(move || loop {
        let events = match inotify.read_events() {
            Ok(events) => events,
            Err(Errno::EINTR) => continue,
            Err(_) => return,
        };

        for event in events {
            if event.mask.intersects(AddWatchFlags::IN_Q_OVERFLOW) {
                // The queue overflowed and events (an OOM modification or a
                // cgroup-removal event) may have been lost. Overflow events
                // are reported with wd == -1, so resynchronize from the
                // current state instead of filtering by watch descriptor.
                if !dir.exists() {
                    return;
                }
                let count = match read_oom_count(&path) {
                    Ok(count) => count,
                    Err(_) => return,
                };

                if count > base {
                    if sender.send(key.clone()).is_err() {
                        return;
                    }
                    base = count;
                }
                continue;
            }

            if event.wd == parent_watch
                && event.name.as_deref() == Some(cgroup_name.as_os_str())
                && event
                    .mask
                    .intersects(AddWatchFlags::IN_DELETE | AddWatchFlags::IN_MOVED_FROM)
            {
                return;
            }

            if event
                .mask
                .intersects(AddWatchFlags::IN_IGNORED | AddWatchFlags::IN_UNMOUNT)
            {
                return;
            }

            if event.wd == events_watch && event.mask.intersects(AddWatchFlags::IN_MODIFY) {
                let count = match read_oom_count(&path) {
                    Ok(count) => count,
                    Err(_) => return,
                };

                if count > base {
                    if sender.send(key.clone()).is_err() {
                        return;
                    }
                    base = count;
                }
            }
        }
    });
    Ok(receiver)
}

// notify_on_oom returns channel on which you can expect event about OOM,
// if cgroup was destroyed without OOM this channel will be closed.
pub fn notify_on_oom_v1(key: &str, dir: &Path) -> Result<Receiver<String>> {
    register_memory_event(key, dir, "memory.oom_control", "")
}

// level is one of "low", "medium", or "critical"
pub fn notify_memory_pressure(key: &str, dir: &Path, level: &str) -> Result<Receiver<String>> {
    if level != "low" && level != "medium" && level != "critical" {
        return Err(Error::from_string(format!(
            "invalid pressure level {}",
            level
        )));
    }

    register_memory_event(key, dir, "memory.pressure_level", level)
}

fn register_memory_event(
    key: &str,
    cg_dir: &Path,
    event_name: &str,
    arg: &str,
) -> Result<Receiver<String>> {
    let path = cg_dir.join(event_name);
    let event_file = File::open(path.clone())
        .map_err(|e| Error::with_cause(ReadFailed(path.display().to_string()), e))?;

    let eventfd = EventFd::from_flags(EfdFlags::EFD_CLOEXEC)
        .map_err(|e| Error::with_cause(ReadFailed("eventfd".to_string()), e))?;

    let event_control_path = cg_dir.join("cgroup.event_control");
    let data = if arg.is_empty() {
        format!("{} {}", eventfd.as_raw_fd(), event_file.as_raw_fd())
    } else {
        format!("{} {} {}", eventfd.as_raw_fd(), event_file.as_raw_fd(), arg)
    };

    // write to file and set mode to 0700(FIXME)
    fs::write(&event_control_path, data.clone()).map_err(|e| {
        Error::with_cause(
            WriteFailed(event_control_path.display().to_string(), data),
            e,
        )
    })?;

    let (sender, receiver) = mpsc::channel();
    let key = key.to_string();

    thread::spawn(move || {
        loop {
            if eventfd.read().is_err() {
                return;
            }

            // When a cgroup is destroyed, an event is sent to eventfd.
            // So if the control path is gone, return instead of notifying.
            if !Path::new(&event_control_path).exists() {
                return;
            }
            sender.send(key.clone()).unwrap();
        }
    });

    Ok(receiver)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cgroups-rs-events-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Overwrite a watched fixture in place. Truncating (fs::write) would
    // itself emit an IN_MODIFY that the watcher may observe while the file
    // is empty, failing read_oom_count and disconnecting the channel.
    // The replacement must have the same length as the current contents.
    fn write_no_trunc(path: &Path, contents: &str) {
        let mut file = OpenOptions::new()
            .write(true)
            .truncate(false)
            .open(path)
            .unwrap();
        assert_eq!(file.metadata().unwrap().len(), contents.len() as u64);
        file.write_all(contents.as_bytes()).unwrap();
    }

    #[test]
    fn test_read_oom_count() {
        let dir = test_dir();
        let path = dir.join("memory.events");

        // full format, including the newer sock_throttled line
        fs::write(
            &path,
            "low 0\nhigh 0\nmax 0\noom 3\noom_kill 0\noom_group_kill 0\nsock_throttled 0\n",
        )
        .unwrap();
        assert_eq!(read_oom_count(&path).unwrap(), 3);

        // line order must not matter, and missing lines are fine (older kernels)
        fs::write(&path, "oom 7\nlow 0\n").unwrap();
        assert_eq!(read_oom_count(&path).unwrap(), 7);

        // key absent -> error
        fs::write(&path, "low 0\nhigh 0\n").unwrap();
        assert!(read_oom_count(&path).is_err());

        // file absent -> error
        assert!(read_oom_count(&dir.join("nope")).is_err());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_notify_on_oom_v2() {
        let dir = test_dir();
        let events = dir.join("memory.events");
        fs::write(&events, "low 0\nhigh 0\nmax 0\noom 5\noom_kill 0\n").unwrap();

        let rx = notify_on_oom_v2("test-key", &dir).unwrap();

        // Pre-existing OOMs are the baseline, not notified.
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        // A kill-count change without a new OOM must not notify.
        write_no_trunc(&events, "low 0\nhigh 0\nmax 0\noom 5\noom_kill 1\n");
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        // One new OOM produces exactly one notification.
        write_no_trunc(&events, "low 0\nhigh 0\nmax 0\noom 6\noom_kill 1\n");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "test-key");

        // no duplicate flood while the count stays the same
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        // A second OOM is notified again.
        write_no_trunc(&events, "low 0\nhigh 0\nmax 0\noom 7\noom_kill 1\n");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "test-key");

        // Destroying the cgroup closes the channel.
        fs::remove_dir_all(&dir).unwrap();
        let closed = (0..20).any(|_| {
            matches!(
                rx.recv_timeout(Duration::from_millis(200)),
                Err(mpsc::RecvTimeoutError::Disconnected)
            )
        });
        assert!(closed);
    }

    #[test]
    fn test_notify_on_oom_v2_closes_when_cgroup_moves() {
        let dir = test_dir();
        let sibling_dir = dir.with_extension("sibling");
        let moved_dir = dir.with_extension("moved");
        let events = dir.join("memory.events");
        fs::write(&events, "low 0\nhigh 0\nmax 0\noom 0\noom_kill 0\n").unwrap();

        let rx = notify_on_oom_v2("test-key", &dir).unwrap();

        fs::create_dir(&sibling_dir).unwrap();
        fs::remove_dir(&sibling_dir).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        fs::rename(&dir, &moved_dir).unwrap();

        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );

        fs::remove_dir_all(&moved_dir).unwrap();
    }
}
