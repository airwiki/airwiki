#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
    echo "usage: azure-install.sh <release-binary> <public-ipv4>" >&2
    exit 1
fi

readonly release_binary="$1"
readonly public_ipv4="$2"
readonly service_user="airwiki"
readonly state_directory="/var/lib/airwiki-federation"
readonly installed_binary="/usr/local/bin/airwiki-federation-index"

if [ ! -f "${release_binary}" ]; then
    echo "release binary does not exist" >&2
    exit 1
fi
case "${public_ipv4}" in
    *[!0-9.]* | "")
        echo "public IPv4 address is invalid" >&2
        exit 1
        ;;
esac

if ! id "${service_user}" >/dev/null 2>&1; then
    useradd \
        --system \
        --home-dir "${state_directory}" \
        --shell /usr/sbin/nologin \
        "${service_user}"
fi

install -d -m 0750 -o "${service_user}" -g "${service_user}" "${state_directory}"
install -m 0755 -o root -g root "${release_binary}" "${installed_binary}"

cat > /etc/systemd/system/airwiki-federation-index.service <<EOF
[Unit]
Description=AirWiki experimental public federation index and relay
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=${service_user}
Group=${service_user}
ExecStart=${installed_binary} ${state_directory}/index.db --external-address /ip4/${public_ipv4}/tcp/42042 --external-address /ip4/${public_ipv4}/udp/42042/quic-v1 /ip4/0.0.0.0/tcp/42042 /ip4/0.0.0.0/udp/42042/quic-v1
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
ReadWritePaths=${state_directory}
RestrictAddressFamilies=AF_INET AF_INET6 AF_NETLINK AF_UNIX
RestrictRealtime=true
SystemCallArchitectures=native
UMask=0027

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now airwiki-federation-index.service
