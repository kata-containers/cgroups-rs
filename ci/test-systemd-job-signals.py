#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0 OR MIT
"""Run the job notification regression against a private dbus-broker."""

import argparse
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time


TEST = "systemd::dbus::client::tests::test_start_job_signal_with_broker"
POLICY = """<!DOCTYPE busconfig PUBLIC
 "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>{address}</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow user="*"/>
    <allow own="*"/>
    <allow send_destination="*"/>
    <allow receive_sender="*"/>
  </policy>
</busconfig>
"""


def broker_child(launcher, config, fd):
    fd = int(fd)
    if fd != 3:
        os.dup2(fd, 3)
        os.close(fd)
    os.set_inheritable(3, True)
    env = dict(os.environ, LISTEN_PID=str(os.getpid()), LISTEN_FDS="1")
    os.execve(launcher, [launcher, "--scope", "user", "--config-file", config], env)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("test_binary", type=Path)
    parser.add_argument("--broker-launcher", default="dbus-broker-launch")
    args = parser.parse_args()
    binary = args.test_binary.resolve(strict=True)
    listing = subprocess.run(
        [str(binary), "--list", "--ignored"],
        check=True, capture_output=True, text=True,
    )
    if f"{TEST}: test" not in listing.stdout.splitlines():
        parser.error(f"test binary does not contain {TEST}")
    executable = shutil.which(args.broker_launcher)
    if executable is None:
        parser.error("missing dbus-broker-launch executable")

    with tempfile.TemporaryDirectory(prefix="cgroups-job-bus-") as directory:
        directory = Path(directory)
        config = directory / "bus.conf"
        address = f"unix:path={directory / 'bus'}"
        config.write_text(POLICY.format(address=address))
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(directory / "bus"))
        listener.listen()
        command = [
            sys.executable, str(Path(__file__).resolve()), "--broker-child",
            executable, str(config), str(listener.fileno()),
        ]

        with (directory / "bus.log").open("w+") as log:
            bus = subprocess.Popen(
                command, pass_fds=(listener.fileno(),), stdout=log, stderr=log,
            )
            listener.close()
            try:
                for _ in range(100):
                    if bus.poll() is not None:
                        raise RuntimeError(f"private broker exited: {bus.returncode}")
                    probe = subprocess.run(
                        ["busctl", f"--address={address}", "list"],
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                        timeout=2, check=False,
                    )
                    if probe.returncode == 0:
                        break
                    time.sleep(0.05)
                else:
                    raise RuntimeError("private broker did not become ready")

                env = dict(os.environ, DBUS_SYSTEM_BUS_ADDRESS=address,
                           CGROUPS_RS_TEST_PRIVATE_BUS="1")
                result = subprocess.run(
                    [str(binary), "--ignored", "--exact", TEST, "--nocapture"],
                    env=env, timeout=45, check=False,
                )
                return result.returncode
            finally:
                bus.terminate()
                try:
                    bus.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    bus.kill()
                    bus.wait()
                log.seek(0)
                sys.stderr.write(log.read())


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--broker-child":
        broker_child(*sys.argv[2:])
    else:
        sys.exit(main())
