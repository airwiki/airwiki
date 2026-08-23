#!/bin/sh
set -eu

if [ "$#" -lt 2 ] || [ "$#" -gt 3 ]; then
    echo "usage: AIRWIKI_FEDERATION_BINARY_SHA256=<sha256> azure-install.sh <release-binary> <public-ipv4> [instance-count]" >&2
    exit 1
fi

readonly release_binary="$1"
readonly public_ipv4="$2"
readonly instance_count="${3:-1}"
readonly expected_sha256="${AIRWIKI_FEDERATION_BINARY_SHA256:-}"
readonly service_user="airwiki"
readonly state_directory="/var/lib/airwiki-federation"
readonly installed_binary="/usr/local/bin/airwiki-federation-index"

is_valid_ipv4() {
    printf '%s\n' "$1" | awk -F. '
        NF != 4 { exit 1 }
        {
            for (octet = 1; octet <= 4; octet++) {
                if ($octet !~ /^[0-9]+$/ || length($octet) > 3 || $octet ~ /^0[0-9]+$/ || $octet > 255) {
                    exit 1
                }
            }
        }
    '
}

if ! is_valid_ipv4 "${public_ipv4}"; then
    echo "public IPv4 address is invalid" >&2
    exit 1
fi
if [ -L "${release_binary}" ] || [ ! -f "${release_binary}" ] || [ ! -x "${release_binary}" ]; then
    echo "release binary must be an executable regular non-symlink file" >&2
    exit 1
fi
if [ "${#expected_sha256}" -ne 64 ]; then
    echo "AIRWIKI_FEDERATION_BINARY_SHA256 must contain exactly 64 lowercase hexadecimal characters" >&2
    exit 1
fi
case "${expected_sha256}" in
    *[!0-9a-f]*)
        echo "AIRWIKI_FEDERATION_BINARY_SHA256 must contain exactly 64 lowercase hexadecimal characters" >&2
        exit 1
        ;;
esac
if [ "$(uname -m)" != "x86_64" ]; then
    echo "the Azure validation host must be x86_64" >&2
    exit 1
fi
if ! command -v file >/dev/null 2>&1 || ! command -v sha256sum >/dev/null 2>&1; then
    echo "file and sha256sum are required to verify the release binary" >&2
    exit 1
fi
staged_binary="$(mktemp /usr/local/bin/.airwiki-federation-index.XXXXXX)"
if [ -z "${staged_binary}" ]; then
    echo "could not create the staged federation-index binary" >&2
    exit 1
fi
readonly staged_binary
trap 'rm -f -- "${staged_binary}"' EXIT HUP INT TERM
install -m 0755 -o root -g root "${release_binary}" "${staged_binary}"
staged_sha256="$(sha256sum -- "${staged_binary}" | awk '{print $1}')"
readonly staged_sha256
if [ "${staged_sha256}" != "${expected_sha256}" ]; then
    echo "release binary checksum does not match the approved candidate" >&2
    exit 1
fi
binary_description="$(LC_ALL=C file -b -- "${staged_binary}")"
readonly binary_description
case "${binary_description}" in
    "ELF 64-bit LSB executable, x86-64,"* | "ELF 64-bit LSB pie executable, x86-64,"*) ;;
    *)
        echo "release binary architecture or format is invalid" >&2
        exit 1
        ;;
esac

case "${instance_count}" in
    1 | 2 | 3) ;;
    *)
        echo "instance count must be between 1 and 3" >&2
        exit 1
        ;;
esac

preflight_instance=1
while [ "${preflight_instance}" -le "${instance_count}" ]; do
    preflight_port=$((42042 + (preflight_instance - 1) * 2))
    "${staged_binary}" \
        --validate-external-address \
        "/ip4/${public_ipv4}/tcp/${preflight_port}" \
        "/ip4/${public_ipv4}/udp/${preflight_port}/quic-v1"
    preflight_instance=$((preflight_instance + 1))
done

stop_managed_service() {
    service_name="$1"
    load_state="$(systemctl show --property=LoadState --value "${service_name}")" || {
        echo "could not inspect ${service_name}; refusing to replace a potentially running relay" >&2
        exit 1
    }
    if [ "${load_state}" != "not-found" ]; then
        systemctl disable --now "${service_name}"
    fi
}

if ! id "${service_user}" >/dev/null 2>&1; then
    useradd \
        --system \
        --home-dir "${state_directory}" \
        --shell /usr/sbin/nologin \
        "${service_user}"
fi

install -d -m 0750 -o "${service_user}" -g "${service_user}" "${state_directory}"

install_rollback_directory="$(mktemp -d /var/tmp/airwiki-federation-install.XXXXXX)"
if [ -z "${install_rollback_directory}" ]; then
    echo "could not create the federation-index rollback directory" >&2
    exit 1
fi
readonly install_rollback_directory
mutation_started=0
install_succeeded=0

restore_previous_install() {
    restore_failed=0
    for restore_service in \
        airwiki-federation-index-1.service \
        airwiki-federation-index-2.service \
        airwiki-federation-index-3.service \
        airwiki-federation-index.service; do
        if [ -e "/etc/systemd/system/${restore_service}" ] &&
            ! systemctl disable --now "${restore_service}" >/dev/null 2>&1; then
            restore_failed=1
        fi
        rm -f "/etc/systemd/system/${restore_service}" || restore_failed=1
    done
    if [ -f "${install_rollback_directory}/installed-binary" ]; then
        install -m 0755 -o root -g root \
            "${install_rollback_directory}/installed-binary" \
            "${installed_binary}" || restore_failed=1
    else
        rm -f "${installed_binary}" || restore_failed=1
    fi
    for restore_service in \
        airwiki-federation-index-1.service \
        airwiki-federation-index-2.service \
        airwiki-federation-index-3.service \
        airwiki-federation-index.service; do
        if [ -f "${install_rollback_directory}/${restore_service}" ]; then
            install -m 0644 -o root -g root \
                "${install_rollback_directory}/${restore_service}" \
                "/etc/systemd/system/${restore_service}" || restore_failed=1
        fi
    done
    systemctl daemon-reload || restore_failed=1
    for restore_service in \
        airwiki-federation-index-1.service \
        airwiki-federation-index-2.service \
        airwiki-federation-index-3.service \
        airwiki-federation-index.service; do
        if [ -f "${install_rollback_directory}/${restore_service}.enabled" ] &&
            ! systemctl enable "${restore_service}" >/dev/null 2>&1; then
            restore_failed=1
        fi
        if [ -f "${install_rollback_directory}/${restore_service}.active" ] &&
            ! systemctl start "${restore_service}"; then
            restore_failed=1
        fi
    done
    if [ "${restore_failed}" -ne 0 ]; then
        echo "INSTALL ROLLBACK INCOMPLETE: inspect the selected beta node before reuse" >&2
        return 1
    fi
    echo "failed candidate removed and previous federation-index install restored" >&2
}

cleanup_install() {
    install_exit_code="$?"
    trap - 0 HUP INT TERM
    if [ "${mutation_started}" -eq 1 ] && [ "${install_succeeded}" -eq 0 ]; then
        if ! restore_previous_install; then
            install_exit_code=1
        fi
    fi
    rm -f -- "${staged_binary}" || install_exit_code=1
    rm -rf -- "${install_rollback_directory}" || install_exit_code=1
    exit "${install_exit_code}"
}
trap cleanup_install 0
trap 'exit 1' HUP INT TERM

if [ -L "${installed_binary}" ]; then
    echo "refusing to replace a symlinked federation-index binary" >&2
    exit 1
fi
if [ -e "${installed_binary}" ] && [ ! -f "${installed_binary}" ]; then
    echo "refusing to replace an unexpected federation-index binary path" >&2
    exit 1
fi
if [ -f "${installed_binary}" ]; then
    install -m 0755 -o root -g root \
        "${installed_binary}" \
        "${install_rollback_directory}/installed-binary"
fi
for backup_service in \
    airwiki-federation-index-1.service \
    airwiki-federation-index-2.service \
    airwiki-federation-index-3.service \
    airwiki-federation-index.service; do
    backup_unit="/etc/systemd/system/${backup_service}"
    if [ -L "${backup_unit}" ]; then
        echo "refusing to replace a symlinked federation-index unit" >&2
        exit 1
    fi
    if [ -e "${backup_unit}" ] && [ ! -f "${backup_unit}" ]; then
        echo "refusing to replace an unexpected federation-index unit path" >&2
        exit 1
    fi
    if [ -f "${backup_unit}" ]; then
        install -m 0644 -o root -g root \
            "${backup_unit}" \
            "${install_rollback_directory}/${backup_service}"
        if systemctl is-enabled --quiet "${backup_service}"; then
            : >"${install_rollback_directory}/${backup_service}.enabled"
        fi
        if systemctl is-active --quiet "${backup_service}"; then
            : >"${install_rollback_directory}/${backup_service}.active"
        fi
    fi
done
mutation_started=1

for stale_instance in 1 2 3; do
    stale_service="airwiki-federation-index-${stale_instance}.service"
    stop_managed_service "${stale_service}"
    rm -f "/etc/systemd/system/${stale_service}"
done
stop_managed_service airwiki-federation-index.service
rm -f /etc/systemd/system/airwiki-federation-index.service

mv -f -- "${staged_binary}" "${installed_binary}"
readonly installed_sha256="$(sha256sum -- "${installed_binary}" | awk '{print $1}')"
if [ "${installed_sha256}" != "${expected_sha256}" ]; then
    echo "installed release binary checksum is invalid" >&2
    exit 1
fi

instance=1
while [ "${instance}" -le "${instance_count}" ]; do
    port=$((42042 + (instance - 1) * 2))
    instance_directory="${state_directory}/index-${instance}"
    service_name="airwiki-federation-index-${instance}.service"
    install -d -m 0750 -o "${service_user}" -g "${service_user}" "${instance_directory}"
    cat > "/etc/systemd/system/${service_name}" <<EOF
[Unit]
Description=AirWiki experimental public federation index and relay ${instance}
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=60s
StartLimitBurst=5

[Service]
Type=simple
User=${service_user}
Group=${service_user}
ExecStart=${installed_binary} ${instance_directory}/index.db --external-address /ip4/${public_ipv4}/tcp/${port} --external-address /ip4/${public_ipv4}/udp/${port}/quic-v1 /ip4/0.0.0.0/tcp/${port} /ip4/0.0.0.0/udp/${port}/quic-v1
Restart=on-failure
RestartSec=5s
TimeoutStopSec=20s
Environment=RUST_LOG=warn,airwiki_federation_index=info,airwiki_network=info,libp2p=off,libp2p_swarm=off
AmbientCapabilities=
CapabilityBoundingSet=
DevicePolicy=closed
LimitNOFILE=4096
LockPersonality=true
MemoryDenyWriteExecute=true
MemoryMax=768M
NoNewPrivileges=true
PrivateDevices=true
PrivateTmp=true
ProcSubset=pid
ProtectControlGroups=true
ProtectClock=true
ProtectHome=true
ProtectHostname=true
ProtectKernelLogs=true
ProtectKernelModules=true
ProtectKernelTunables=true
ProtectProc=invisible
ProtectSystem=strict
ReadWritePaths=${instance_directory}
RemoveIPC=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_NETLINK AF_UNIX
RestrictNamespaces=true
RestrictRealtime=true
RestrictSUIDSGID=true
StandardError=journal
StandardOutput=journal
SystemCallArchitectures=native
SystemCallErrorNumber=EPERM
SystemCallFilter=@system-service
TasksMax=128
UMask=0027

[Install]
WantedBy=multi-user.target
EOF
    instance=$((instance + 1))
done

systemctl daemon-reload
instance=1
while [ "${instance}" -le "${instance_count}" ]; do
    service_name="airwiki-federation-index-${instance}.service"
    systemctl enable --now "${service_name}"
    if ! systemctl is-active --quiet "${service_name}"; then
        echo "${service_name} did not become active" >&2
        exit 1
    fi
    instance=$((instance + 1))
done
install_succeeded=1
