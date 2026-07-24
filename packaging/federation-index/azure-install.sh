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
readonly staged_binary="$(mktemp /usr/local/bin/.airwiki-federation-index.XXXXXX)"
trap 'rm -f -- "${staged_binary}"' EXIT HUP INT TERM
install -m 0755 -o root -g root "${release_binary}" "${staged_binary}"
readonly staged_sha256="$(sha256sum -- "${staged_binary}" | awk '{print $1}')"
if [ "${staged_sha256}" != "${expected_sha256}" ]; then
    echo "release binary checksum does not match the approved candidate" >&2
    exit 1
fi
readonly binary_description="$(LC_ALL=C file -b -- "${staged_binary}")"
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

[Service]
Type=simple
User=${service_user}
Group=${service_user}
ExecStart=${installed_binary} ${instance_directory}/index.db --external-address /ip4/${public_ipv4}/tcp/${port} --external-address /ip4/${public_ipv4}/udp/${port}/quic-v1 /ip4/0.0.0.0/tcp/${port} /ip4/0.0.0.0/udp/${port}/quic-v1
Restart=on-failure
RestartSec=5s
TimeoutStopSec=20s
Environment=RUST_LOG=warn,airwiki_federation_index=info,airwiki_network=info,libp2p=off,libp2p_swarm=off
NoNewPrivileges=true
PrivateDevices=true
PrivateTmp=true
ProtectControlGroups=true
ProtectHome=true
ProtectKernelLogs=true
ProtectKernelModules=true
ProtectKernelTunables=true
ProtectSystem=strict
ReadWritePaths=${instance_directory}
RestrictAddressFamilies=AF_INET AF_INET6 AF_NETLINK AF_UNIX
RestrictRealtime=true
SystemCallArchitectures=native
UMask=0027

[Install]
WantedBy=multi-user.target
EOF
    instance=$((instance + 1))
done

systemctl daemon-reload
instance=1
while [ "${instance}" -le "${instance_count}" ]; do
    systemctl enable --now "airwiki-federation-index-${instance}.service"
    instance=$((instance + 1))
done
