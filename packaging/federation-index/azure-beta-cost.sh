#!/bin/sh
set -eu

readonly ceiling_usd="50.00"
readonly monthly_hours="730"
readonly planned_egress_gb="250"
readonly included_egress_gb="100"
readonly disk_operation_allowance_usd="0.40"
readonly availability_alert_allowance_usd="0.20"
readonly prices_endpoint="https://prices.azure.com/api/retail/prices"

if [ "$#" -gt 1 ] || { [ "$#" -eq 1 ] && [ "$1" != "--check" ]; }; then
    echo "usage: azure-beta-cost.sh [--check]" >&2
    exit 1
fi
if ! command -v curl >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
    echo "curl and jq are required for the Azure retail-price check" >&2
    exit 1
fi

retail_prices() {
    filter="$1"
    curl --fail --silent --show-error --get "${prices_endpoint}" \
        --connect-timeout 10 \
        --max-time 30 \
        --retry 2 \
        --retry-all-errors \
        --data-urlencode "currencyCode=USD" \
        --data-urlencode "\$filter=${filter}"
}

single_price() {
    values="$(jq -r "$1" | sort -u)"
    if [ -z "${values}" ] || [ "$(printf '%s\n' "${values}" | wc -l | tr -d ' ')" -ne 1 ]; then
        echo "Azure retail pricing did not return one unambiguous price" >&2
        exit 1
    fi
    printf '%s\n' "${values}"
}

east_vm_price="$(
    retail_prices "serviceName eq 'Virtual Machines' and armRegionName eq 'eastus' and armSkuName eq 'Standard_B1s' and priceType eq 'Consumption'" |
        single_price '.Items[] | select(.productName | endswith("Windows") | not) | .retailPrice'
)"
west_vm_price="$(
    retail_prices "serviceName eq 'Virtual Machines' and armRegionName eq 'northcentralus' and armSkuName eq 'Standard_B1s' and priceType eq 'Consumption'" |
        single_price '.Items[] | select(.productName | endswith("Windows") | not) | .retailPrice'
)"
east_disk_price="$(
    retail_prices "serviceName eq 'Storage' and armRegionName eq 'eastus' and productName eq 'Standard SSD Managed Disks'" |
        single_price '.Items[] | select(.skuName == "E4 LRS" and .meterName == "E4 LRS Disk") | .retailPrice'
)"
west_disk_price="$(
    retail_prices "serviceName eq 'Storage' and armRegionName eq 'northcentralus' and productName eq 'Standard SSD Managed Disks'" |
        single_price '.Items[] | select(.skuName == "E4 LRS" and .meterName == "E4 LRS Disk") | .retailPrice'
)"
east_ip_price="$(
    retail_prices "serviceName eq 'Virtual Network' and armRegionName eq 'eastus'" |
        single_price '.Items[] | select(.productName == "IP Addresses" and .skuName == "Standard" and .meterName == "Standard IPv4 Static Public IP") | .retailPrice'
)"
west_ip_price="$(
    retail_prices "serviceName eq 'Virtual Network' and armRegionName eq 'northcentralus'" |
        single_price '.Items[] | select(.productName == "IP Addresses" and .skuName == "Standard" and .meterName == "Standard IPv4 Static Public IP") | .retailPrice'
)"
egress_price="$(
    retail_prices "serviceName eq 'Bandwidth' and armRegionName eq 'eastus'" |
        single_price '.Items[] | select(.productName == "Rtn Preference: MGN" and .skuName == "Standard" and .meterName == "Standard Data Transfer Out" and .tierMinimumUnits == 100) | .retailPrice'
)"

estimate="$(
    awk \
        -v east_vm="${east_vm_price}" \
        -v west_vm="${west_vm_price}" \
        -v east_disk="${east_disk_price}" \
        -v west_disk="${west_disk_price}" \
        -v east_ip="${east_ip_price}" \
        -v west_ip="${west_ip_price}" \
        -v hours="${monthly_hours}" \
        -v egress_price="${egress_price}" \
        -v egress_gb="${planned_egress_gb}" \
        -v included_gb="${included_egress_gb}" \
        -v disk_ops="${disk_operation_allowance_usd}" \
        -v alerts="${availability_alert_allowance_usd}" \
        'BEGIN {
            base = ((east_vm + west_vm + east_ip + west_ip) * hours) + east_disk + west_disk;
            transfer = (egress_gb - included_gb) * egress_price;
            printf "%.2f", base + transfer + disk_ops + alerts;
        }'
)"
no_free_tier_estimate="$(
    awk \
        -v east_vm="${east_vm_price}" \
        -v west_vm="${west_vm_price}" \
        -v east_disk="${east_disk_price}" \
        -v west_disk="${west_disk_price}" \
        -v east_ip="${east_ip_price}" \
        -v west_ip="${west_ip_price}" \
        -v hours="${monthly_hours}" \
        -v egress_price="${egress_price}" \
        -v egress_gb="${planned_egress_gb}" \
        -v disk_ops="${disk_operation_allowance_usd}" \
        -v alerts="${availability_alert_allowance_usd}" \
        'BEGIN {
            base = ((east_vm + west_vm + east_ip + west_ip) * hours) + east_disk + west_disk;
            printf "%.2f", base + (egress_gb * egress_price) + disk_ops + alerts;
        }'
)"

printf '%s\n' \
    "AirWiki public federation beta v1 monthly ceiling (USD)" \
    "  two Standard_B1s Linux VMs, 730 h: $(awk -v a="${east_vm_price}" -v b="${west_vm_price}" -v h="${monthly_hours}" 'BEGIN { printf "%.2f", (a+b)*h }')" \
    "  two Standard SSD E4 OS disks: $(awk -v a="${east_disk_price}" -v b="${west_disk_price}" 'BEGIN { printf "%.2f", a+b }')" \
    "  two Standard static IPv4 addresses, 730 h: $(awk -v a="${east_ip_price}" -v b="${west_ip_price}" -v h="${monthly_hours}" 'BEGIN { printf "%.2f", (a+b)*h }')" \
    "  ${planned_egress_gb} GB total egress (${included_egress_gb} GB included): $(awk -v total="${planned_egress_gb}" -v included="${included_egress_gb}" -v rate="${egress_price}" 'BEGIN { printf "%.2f", (total-included)*rate }')" \
    "  disk operations allowance: ${disk_operation_allowance_usd}" \
    "  two VM availability-alert allowance: ${availability_alert_allowance_usd}" \
    "  realistic planned total: ${estimate}" \
    "  conservative total if the account free tier is already consumed: ${no_free_tier_estimate}" \
    "  configured monthly budget ceiling: ${ceiling_usd}"

if ! awk -v estimate="${no_free_tier_estimate}" -v ceiling="${ceiling_usd}" 'BEGIN { exit !(estimate <= ceiling) }'; then
    echo "the current Azure estimate exceeds the approved ceiling" >&2
    exit 1
fi

if [ "${1:-}" = "--check" ]; then
    printf '%s\n' "cost ceiling check: PASS"
fi
