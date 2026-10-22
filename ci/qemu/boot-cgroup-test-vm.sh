#!/usr/bin/env bash
#
# Start an Ubuntu 22.04 VM in an explicit cgroup mode. TCG is the default
# accelerator, so the VM also boots on hosts without KVM. Set QEMU_ACCEL=kvm
# where /dev/kvm is available, as the CI workflow does.

set -euo pipefail

readonly USAGE="usage: boot-cgroup-test-vm.sh STATE_DIR BASE_IMAGE PUBLIC_KEY HIERARCHY"
readonly STATE_DIR="${1:?${USAGE}}"
readonly BASE_IMAGE="${2:?${USAGE}}"
readonly PUBLIC_KEY="${3:?${USAGE}}"
readonly PRIVATE_KEY="${PUBLIC_KEY%.pub}"
readonly HIERARCHY="${4:?${USAGE}}"
readonly SSH_PORT="${CGROUP_VM_SSH_PORT:-2222}"
readonly QEMU_ACCEL="${QEMU_ACCEL:-tcg}"
readonly SSH_USER="runner"

case "${HIERARCHY}" in
    v1)
        unified="0"
        hierarchy_check='
            grep -qw systemd.unified_cgroup_hierarchy=0 /proc/cmdline &&
            test "$(stat -fc %T /sys/fs/cgroup)" != cgroup2fs &&
            test -n "$(findmnt -rn -t cgroup -o TARGET)"
        '
        ;;
    v2)
        unified="1"
        hierarchy_check='
            grep -qw systemd.unified_cgroup_hierarchy=1 /proc/cmdline &&
            test "$(stat -fc %T /sys/fs/cgroup)" = cgroup2fs &&
            test -f /sys/fs/cgroup/cgroup.controllers
        '
        ;;
    *)
        echo "unsupported cgroup hierarchy: ${HIERARCHY}" >&2
        exit 1
        ;;
esac

for command in cloud-localds qemu-img qemu-system-x86_64 ssh; do
    if ! command -v "${command}" >/dev/null; then
        echo "required command is missing: ${command}" >&2
        exit 1
    fi
done

if [[ -e "${STATE_DIR}" ]]; then
    echo "state directory already exists: ${STATE_DIR}" >&2
    exit 1
fi
if [[ ! -f "${BASE_IMAGE}" ]]; then
    echo "base image does not exist: ${BASE_IMAGE}" >&2
    exit 1
fi
if [[ ! -f "${PUBLIC_KEY}" ]]; then
    echo "SSH public key does not exist: ${PUBLIC_KEY}" >&2
    exit 1
fi
if [[ ! -f "${PRIVATE_KEY}" ]]; then
    echo "SSH private key does not exist: ${PRIVATE_KEY}" >&2
    exit 1
fi

mkdir -p "${STATE_DIR}"

readonly DISK="${STATE_DIR}/root.qcow2"
readonly SEED="${STATE_DIR}/seed.qcow2"
readonly USER_DATA="${STATE_DIR}/user-data"
readonly META_DATA="${STATE_DIR}/meta-data"
readonly CONSOLE_LOG="${STATE_DIR}/console.log"
readonly QEMU_LOG="${STATE_DIR}/qemu.log"
readonly PID_FILE="${STATE_DIR}/qemu.pid"

qemu-img create \
    -q \
    -f qcow2 \
    -F qcow2 \
    -b "$(realpath "${BASE_IMAGE}")" \
    "${DISK}" \
    8G

ssh_key="$(<"${PUBLIC_KEY}")"

{
    echo "#cloud-config"
    echo "users:"
    echo "  - default"
    echo "  - name: ${SSH_USER}"
    echo "    groups: [adm, sudo]"
    echo "    shell: /bin/bash"
    echo "    sudo: ALL=(ALL) NOPASSWD:ALL"
    echo "    ssh_authorized_keys:"
    printf '      - %s\n' "${ssh_key}"
    echo "ssh_pwauth: false"
    echo "disable_root: true"
    echo "write_files:"
    echo "  - path: /etc/default/grub.d/99-cgroup-hierarchy.cfg"
    echo "    owner: root:root"
    echo "    permissions: '0644'"
    echo "    content: |"
    echo "      GRUB_CMDLINE_LINUX=\"\${GRUB_CMDLINE_LINUX} systemd.unified_cgroup_hierarchy=${unified}\""
    echo "runcmd:"
    echo "  - [update-grub]"
    echo "  - [touch, /var/lib/cgroup-hierarchy-configured]"
    echo "power_state:"
    echo "  mode: reboot"
    echo "  delay: now"
    echo "  timeout: 120"
    echo "  condition: test -f /var/lib/cgroup-hierarchy-configured"
} >"${USER_DATA}"

{
    echo "instance-id: cgroups-rs-${HIERARCHY}"
    echo "local-hostname: cgroups-rs-${HIERARCHY}"
} >"${META_DATA}"

cloud-localds --disk-format qcow2 "${SEED}" "${USER_DATA}" "${META_DATA}"

case "${QEMU_ACCEL}" in
    tcg)
        accel_args=(-accel "tcg,thread=multi" -cpu max)
        ;;
    kvm)
        if [[ ! -r /dev/kvm || ! -w /dev/kvm ]]; then
            echo "QEMU_ACCEL=kvm requested but /dev/kvm is not accessible" >&2
            exit 1
        fi
        accel_args=(-accel kvm -cpu host)
        ;;
    *)
        echo "unsupported QEMU_ACCEL value: ${QEMU_ACCEL}" >&2
        exit 1
        ;;
esac

firmware_args=()
if [[ -f /usr/share/OVMF/OVMF_CODE.fd && -f /usr/share/OVMF/OVMF_VARS.fd ]]; then
    cp /usr/share/OVMF/OVMF_VARS.fd "${STATE_DIR}/OVMF_VARS.fd"
    firmware_args=(
        -drive "if=pflash,format=raw,readonly=on,file=/usr/share/OVMF/OVMF_CODE.fd"
        -drive "if=pflash,format=raw,file=${STATE_DIR}/OVMF_VARS.fd"
    )
fi

echo "Starting cgroup ${HIERARCHY} guest with QEMU accelerator: ${QEMU_ACCEL}"

qemu-system-x86_64 \
    -name "cgroups-rs-${HIERARCHY}" \
    -machine q35 \
    "${accel_args[@]}" \
    -smp 2 \
    -m 4096 \
    "${firmware_args[@]}" \
    -drive "file=${DISK},if=virtio,format=qcow2,cache=writeback" \
    -drive "file=${SEED},if=virtio,format=qcow2,readonly=on" \
    -device virtio-rng-pci \
    -device virtio-net-pci,netdev=net0 \
    -netdev "user,id=net0,hostfwd=tcp:127.0.0.1:${SSH_PORT}-:22" \
    -display none \
    -monitor none \
    -serial "file:${CONSOLE_LOG}" \
    -D "${QEMU_LOG}" \
    -pidfile "${PID_FILE}" \
    -daemonize

ssh_options=(
    -i "${PRIVATE_KEY}"
    -p "${SSH_PORT}"
    -o BatchMode=yes
    -o ConnectTimeout=5
    -o StrictHostKeyChecking=no
    -o UserKnownHostsFile=/dev/null
)

# The first boot applies the requested hierarchy kernel argument and reboots.
# Waiting for the exact filesystem type prevents that first boot from being
# mistaken for the requested test environment.
deadline=$((SECONDS + 1200))
while ((SECONDS < deadline)); do
    if ! kill -0 "$(<"${PID_FILE}")" 2>/dev/null; then
        echo "QEMU exited before the guest became ready" >&2
        tail -n 200 "${CONSOLE_LOG}" >&2 || true
        exit 1
    fi

    if ssh "${ssh_options[@]}" "${SSH_USER}@127.0.0.1" \
        "${hierarchy_check}" \
        >/dev/null 2>&1; then
        echo "The cgroup ${HIERARCHY} guest is ready"
        exit 0
    fi

    sleep 5
done

echo "timed out waiting for the cgroup ${HIERARCHY} guest" >&2
tail -n 200 "${CONSOLE_LOG}" >&2 || true
exit 1
