#!/bin/sh
set -eu

readonly east_group="airwiki-federation-beta-east"
readonly east_region="eastus"
readonly east_node="awbetaeast"
readonly west_group="airwiki-federation-beta-west"
readonly west_region="westus2"
readonly west_node="awbetawest"
readonly admin_username="airwikiops"
readonly approved_ceiling_usd="50.00"
readonly budget_api_version="2024-08-01"
readonly budget_name="airwiki-federation-beta-v1"
readonly action_group_name="airwiki-beta-operator"
readonly availability_alert_name="airwiki-beta-vm-unavailable"

script_directory="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
readonly script_directory
repository_root="$(git -C "${script_directory}" rev-parse --show-toplevel)"
readonly repository_root
readonly node_template="${script_directory}/azure-beta-node.json"
readonly install_script="${script_directory}/azure-install.sh"
readonly cost_script="${script_directory}/azure-beta-cost.sh"

usage() {
    cat >&2 <<'EOF'
usage:
  azure-beta.sh deploy
  azure-beta.sh replace-node <east|west>
  azure-beta.sh install <x86-64-linux-release-binary> [east|west]
  azure-beta.sh bootstrap <registry-version> <expiry-rfc3339>
  azure-beta.sh revoke-bootstrap <registry-version> <expiry-rfc3339> <east|west>
  azure-beta.sh expired-bootstrap <registry-version>
  azure-beta.sh status
  azure-beta.sh stop-node <east|west>
  azure-beta.sh start-node <east|west>
  azure-beta.sh retire

all Azure commands require:
  AIRWIKI_BETA_SUBSCRIPTION_ID=<approved-subscription-id>

deploy and replace-node require:
  AIRWIKI_BETA_COST_APPROVED_USD=50.00
  AIRWIKI_BETA_APPROVAL_SHA=<exact-clean-commit>
  AIRWIKI_BETA_BUDGET_EMAIL=<operator-email>
  AIRWIKI_BETA_MAINTAINER_CIDR=<single-public-ipv4>/32
  AIRWIKI_BETA_SSH_PUBLIC_KEY_FILE=<ed25519-public-key>

replace-node also requires:
  AIRWIKI_BETA_REPLACE_CONFIRM=replace-<east|west>-beta-node

install requires:
  AIRWIKI_BETA_SSH_PRIVATE_KEY_FILE=<matching-private-key>

retire requires:
  AIRWIKI_BETA_RETIRE_CONFIRM=delete-airwiki-federation-beta-v1
  AIRWIKI_BETA_BOOTSTRAP_RETIRED_VERSION=<higher-registry-version>
EOF
    exit 1
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "$1 is required" >&2
        exit 1
    fi
}

require_azure_session() {
    require_command az
    approved_subscription_id="${AIRWIKI_BETA_SUBSCRIPTION_ID:-}"
    printf '%s\n' "${approved_subscription_id}" | awk -F- '
        NF != 5 ||
        length($1) != 8 ||
        length($2) != 4 ||
        length($3) != 4 ||
        length($4) != 4 ||
        length($5) != 12 { exit 1 }
        {
            for (field = 1; field <= 5; field++) {
                if ($field !~ /^[0-9A-Fa-f]+$/) {
                    exit 1
                }
            }
        }
    ' || {
        echo "AIRWIKI_BETA_SUBSCRIPTION_ID must be the explicitly approved Azure subscription" >&2
        exit 1
    }
    if [ "$(az account show --query state -o tsv 2>/dev/null)" != "Enabled" ]; then
        echo "an enabled Azure CLI session is required" >&2
        exit 1
    fi
    current_subscription_id="$(az account show --query id -o tsv 2>/dev/null)"
    if [ "${current_subscription_id}" != "${approved_subscription_id}" ]; then
        echo "the active Azure subscription does not match AIRWIKI_BETA_SUBSCRIPTION_ID" >&2
        exit 1
    fi
}

validate_email() {
    case "$1" in
        *@*.*) ;;
        *)
            echo "AIRWIKI_BETA_BUDGET_EMAIL is invalid" >&2
            exit 1
            ;;
    esac
    case "$1" in
        *[[:space:]]*)
            echo "AIRWIKI_BETA_BUDGET_EMAIL is invalid" >&2
            exit 1
            ;;
    esac
}

validate_maintainer_cidr() {
    printf '%s\n' "$1" | awk -F/ '
        NF != 2 || $2 != 32 { exit 1 }
        {
            octet_count = split($1, octets, ".");
            if (octet_count != 4) {
                exit 1
            }
            for (i = 1; i <= 4; i++) {
                if (octets[i] !~ /^[0-9]+$/ || length(octets[i]) > 3 ||
                    octets[i] ~ /^0[0-9]+$/ || octets[i] > 255) {
                    exit 1
                }
            }
        }
    ' || {
        echo "AIRWIKI_BETA_MAINTAINER_CIDR must be one canonical public IPv4 /32" >&2
        exit 1
    }
    address="${1%/32}"
    printf '%s\n' "${address}" | awk -F. '
        $1 == 0 ||
        $1 == 10 ||
        ($1 == 100 && $2 >= 64 && $2 <= 127) ||
        $1 == 127 ||
        ($1 == 169 && $2 == 254) ||
        ($1 == 172 && $2 >= 16 && $2 <= 31) ||
        ($1 == 192 && $2 == 0 && $3 == 0) ||
        ($1 == 192 && $2 == 0 && $3 == 2) ||
        ($1 == 192 && $2 == 168) ||
        ($1 == 198 && $2 >= 18 && $2 <= 19) ||
        ($1 == 198 && $2 == 51 && $3 == 100) ||
        ($1 == 203 && $2 == 0 && $3 == 113) ||
        $1 >= 224 { exit 1 }
    ' || {
        echo "AIRWIKI_BETA_MAINTAINER_CIDR must be publicly routable" >&2
        exit 1
    }
}

node_values() {
    case "$1" in
        east) printf '%s\n%s\n%s\n' "${east_group}" "${east_region}" "${east_node}" ;;
        west) printf '%s\n%s\n%s\n' "${west_group}" "${west_region}" "${west_node}" ;;
        *) usage ;;
    esac
}

group_state() {
    group="$1"
    require_command jq
    presence="$(az group exists --name "${group}" -o tsv)" || {
        echo "could not determine beta resource-group presence" >&2
        return 1
    }
    case "${presence}" in
        false)
            printf '%s\n' absent
            return
            ;;
        true) ;;
        *)
            echo "Azure returned an invalid resource-group presence state" >&2
            return 1
            ;;
    esac
    tags="$(
        az group show \
            --name "${group}" \
            --query tags \
            -o json
    )" || {
        echo "could not verify beta resource-group ownership" >&2
        return 1
    }
    if printf '%s\n' "${tags}" |
        jq -e '
            .application == "airwiki" and
            .component == "public-federation-beta-v1"
        ' >/dev/null; then
        printf '%s\n' beta
    else
        printf '%s\n' foreign
    fi
}

require_beta_group() {
    state="$(group_state "$1")"
    if [ "${state}" != "beta" ]; then
        echo "the dedicated, tagged beta resource group is required" >&2
        exit 1
    fi
}

require_beta_groups() {
    east_state="$(group_state "${east_group}")"
    west_state="$(group_state "${west_group}")"
    if [ "${east_state}" != "beta" ] || [ "${west_state}" != "beta" ]; then
        echo "both dedicated, tagged beta resource groups are required" >&2
        exit 1
    fi
}

create_budget() {
    group="$1"
    email="$2"
    subscription_id="$3"
    body_file="$4"
    start_date="$(date -u +%Y-%m-01T00:00:00Z)"
    scope="/subscriptions/${subscription_id}/resourceGroups/${group}"
    budget_uri="https://management.azure.com${scope}/providers/Microsoft.Consumption/budgets/${budget_name}?api-version=${budget_api_version}"

    jq -n \
        --arg email "${email}" \
        --arg start_date "${start_date}" \
        '{
            properties: {
                amount: 25,
                category: "Cost",
                timeGrain: "Monthly",
                timePeriod: {
                    startDate: $start_date,
                    endDate: "2036-12-31T00:00:00Z"
                },
                notifications: {
                    actual_50: {
                        enabled: true,
                        operator: "GreaterThanOrEqualTo",
                        threshold: 50,
                        thresholdType: "Actual",
                        contactEmails: [$email],
                        contactGroups: [],
                        contactRoles: []
                    },
                    actual_75: {
                        enabled: true,
                        operator: "GreaterThanOrEqualTo",
                        threshold: 75,
                        thresholdType: "Actual",
                        contactEmails: [$email],
                        contactGroups: [],
                        contactRoles: []
                    },
                    actual_90: {
                        enabled: true,
                        operator: "GreaterThanOrEqualTo",
                        threshold: 90,
                        thresholdType: "Actual",
                        contactEmails: [$email],
                        contactGroups: [],
                        contactRoles: []
                    },
                    actual_100: {
                        enabled: true,
                        operator: "GreaterThanOrEqualTo",
                        threshold: 100,
                        thresholdType: "Actual",
                        contactEmails: [$email],
                        contactGroups: [],
                        contactRoles: []
                    },
                    forecast_100: {
                        enabled: true,
                        operator: "GreaterThanOrEqualTo",
                        threshold: 100,
                        thresholdType: "Forecasted",
                        contactEmails: [$email],
                        contactGroups: [],
                        contactRoles: []
                    }
                }
            }
        }' >"${body_file}"
    az rest \
        --method put \
        --uri "${budget_uri}" \
        --body "@${body_file}" \
        --output none \
        --only-show-errors
    budget_currency="$(
        az rest \
            --method get \
            --uri "${budget_uri}" \
            --query "properties.currentSpend.unit" \
            -o tsv \
            --only-show-errors
    )"
    if [ "${budget_currency}" != "USD" ]; then
        echo "the Azure billing scope does not expose this budget in USD" >&2
        return 1
    fi
}

rollback_new_groups() {
    echo "provisioning failed; deleting every newly created beta resource group" >&2
    east_state=skipped
    west_state=skipped
    if [ "${rollback_east:-0}" -eq 1 ]; then
        east_state="$(group_state "${east_group}")" || {
            echo "ROLLBACK INCOMPLETE: east target ownership could not be verified" >&2
            return 1
        }
    fi
    if [ "${rollback_west:-0}" -eq 1 ]; then
        west_state="$(group_state "${west_group}")" || {
            echo "ROLLBACK INCOMPLETE: west target ownership could not be verified" >&2
            return 1
        }
    fi
    if [ "${east_state}" = "foreign" ] || [ "${west_state}" = "foreign" ]; then
        echo "ROLLBACK INCOMPLETE: one target resource group is untagged" >&2
        return 1
    fi
    rollback_failed=0
    if [ "${east_state}" = "beta" ] &&
        ! az group delete \
            --name "${east_group}" \
            --yes \
            --output none \
            --only-show-errors; then
        rollback_failed=1
    fi
    if [ "${west_state}" = "beta" ] &&
        ! az group delete \
            --name "${west_group}" \
            --yes \
            --output none \
            --only-show-errors; then
        rollback_failed=1
    fi
    if [ "${rollback_east:-0}" -eq 1 ]; then
        east_state="$(group_state "${east_group}")" || east_state=unknown
        [ "${east_state}" = "absent" ] || rollback_failed=1
    fi
    if [ "${rollback_west:-0}" -eq 1 ]; then
        west_state="$(group_state "${west_group}")" || west_state=unknown
        [ "${west_state}" = "absent" ] || rollback_failed=1
    fi
    if [ "${rollback_failed}" -ne 0 ]; then
        echo "ROLLBACK INCOMPLETE: a beta resource group may remain billable; run status and retire" >&2
        return 1
    fi
    echo "provisioning rollback confirmed every new beta resource group absent" >&2
}

cleanup_failed_deploy() {
    deploy_exit_code="$?"
    trap - 0 HUP INT TERM
    if [ "${deploy_started:-0}" -eq 1 ] && [ "${deploy_succeeded:-0}" -eq 0 ]; then
        if ! rollback_new_groups; then
            deploy_exit_code=1
        fi
    fi
    if [ -n "${budget_body:-}" ]; then
        rm -f -- "${budget_body}"
    fi
    exit "${deploy_exit_code}"
}

deploy_node() {
    group="$1"
    node="$2"
    public_key="$3"
    maintainer_cidr="$4"
    email="$5"

    az deployment group create \
        --resource-group "${group}" \
        --name "public-federation-beta-v1" \
        --template-file "${node_template}" \
        --parameters \
            nodeName="${node}" \
            adminUsername="${admin_username}" \
            adminPublicKey="${public_key}" \
            maintainerCidr="${maintainer_cidr}" \
        --output none \
        --only-show-errors

    az monitor action-group create \
        --resource-group "${group}" \
        --name "${action_group_name}" \
        --short-name "aw-beta" \
        --action email beta-operator "${email}" usecommonalertschema \
        --tags application=airwiki component=public-federation-beta-v1 \
        --output none \
        --only-show-errors
    vm_id="$(az vm show --resource-group "${group}" --name "${node}" --query id -o tsv)"
    action_group_id="$(
        az monitor action-group show \
            --resource-group "${group}" \
            --name "${action_group_name}" \
            --query id \
            -o tsv
    )"
    az monitor metrics alert create \
        --resource-group "${group}" \
        --name "${availability_alert_name}" \
        --scopes "${vm_id}" \
        --condition "min VmAvailabilityMetric < 1" \
        --window-size 5m \
        --evaluation-frequency 1m \
        --severity 1 \
        --description "AirWiki beta federation VM availability dropped below one." \
        --action "${action_group_id}" \
        --tags application=airwiki component=public-federation-beta-v1 \
        --output none \
        --only-show-errors
}

load_approved_deployment_inputs() {
    approved_cost="${AIRWIKI_BETA_COST_APPROVED_USD:-}"
    approved_sha="${AIRWIKI_BETA_APPROVAL_SHA:-}"
    email="${AIRWIKI_BETA_BUDGET_EMAIL:-}"
    maintainer_cidr="${AIRWIKI_BETA_MAINTAINER_CIDR:-}"
    public_key_file="${AIRWIKI_BETA_SSH_PUBLIC_KEY_FILE:-}"

    if [ "${approved_cost}" != "${approved_ceiling_usd}" ]; then
        echo "explicit approval of the USD ${approved_ceiling_usd} monthly ceiling is required" >&2
        exit 1
    fi
    current_sha="$(git -C "${repository_root}" rev-parse HEAD)"
    if [ -z "${approved_sha}" ] || [ "${approved_sha}" != "${current_sha}" ]; then
        echo "AIRWIKI_BETA_APPROVAL_SHA must equal the exact candidate commit" >&2
        exit 1
    fi
    if [ -n "$(
        git -C "${repository_root}" status \
            --porcelain \
            --untracked-files=normal
    )" ]; then
        echo "provisioning requires a clean worktree at the approved commit" >&2
        exit 1
    fi
    validate_email "${email}"
    validate_maintainer_cidr "${maintainer_cidr}"
    if [ -z "${public_key_file}" ] || [ -L "${public_key_file}" ] || [ ! -f "${public_key_file}" ]; then
        echo "AIRWIKI_BETA_SSH_PUBLIC_KEY_FILE must be a regular non-symlink file" >&2
        exit 1
    fi
    public_key="$(sed -n '1p' "${public_key_file}")"
    case "${public_key}" in
        ssh-ed25519\ *) ;;
        *)
            echo "the beta operator key must be ssh-ed25519" >&2
            exit 1
            ;;
    esac
    if [ "$(wc -l <"${public_key_file}" | tr -d ' ')" -ne 1 ]; then
        echo "the beta operator public-key file must contain exactly one line" >&2
        exit 1
    fi
    subscription_id="${AIRWIKI_BETA_SUBSCRIPTION_ID}"
}

provision_node_group() {
    group="$1"
    region="$2"
    node="$3"
    az group create \
        --name "${group}" \
        --location "${region}" \
        --tags application=airwiki component=public-federation-beta-v1 \
        --output none \
        --only-show-errors
    create_budget "${group}" "${email}" "${subscription_id}" "${budget_body}"
    deploy_node \
        "${group}" "${node}" \
        "${public_key}" "${maintainer_cidr}" "${email}"
}

node_service_is_active() {
    group="$1"
    node="$2"
    power_state="$(
        az vm get-instance-view \
            --resource-group "${group}" \
            --name "${node}" \
            --query "instanceView.statuses[?starts_with(code, 'PowerState/')].code | [0]" \
            -o tsv
    )"
    [ "${power_state}" = "PowerState/running" ] || return 1
    message="$(
        az vm run-command invoke \
            --resource-group "${group}" \
            --name "${node}" \
            --command-id RunShellScript \
            --scripts "systemctl is-active airwiki-federation-index-1.service 2>/dev/null || true" \
            --query "value[0].message" \
            -o tsv
    )"
    [ "$(printf '%s\n' "${message}" | sed -n '/^active$/p' | head -n 1)" = "active" ]
}

deploy() {
    require_azure_session
    require_command jq
    require_command git
    load_approved_deployment_inputs
    east_state="$(group_state "${east_group}")"
    west_state="$(group_state "${west_group}")"
    if [ "${east_state}" != "absent" ] || [ "${west_state}" != "absent" ]; then
        echo "a dedicated beta resource group already exists; refusing to adopt or overwrite it" >&2
        exit 1
    fi

    "${cost_script}" --check
    budget_body="$(mktemp "${TMPDIR:-/tmp}/airwiki-beta-budget.XXXXXX")"
    deploy_started=1
    deploy_succeeded=0
    rollback_east=1
    rollback_west=1
    trap cleanup_failed_deploy 0
    trap 'exit 1' HUP INT TERM

    provision_node_group "${east_group}" "${east_region}" "${east_node}"
    provision_node_group "${west_group}" "${west_region}" "${west_node}"
    deploy_succeeded=1
    trap - 0 HUP INT TERM
    rm -f -- "${budget_body}"
    budget_body=
    printf '%s\n' "two independent beta nodes deployed with budgets and availability alerts"
}

replace_node() {
    [ "$#" -eq 1 ] || usage
    side="$1"
    values="$(node_values "${side}")"
    group="$(printf '%s\n' "${values}" | sed -n '1p')"
    region="$(printf '%s\n' "${values}" | sed -n '2p')"
    node="$(printf '%s\n' "${values}" | sed -n '3p')"
    require_azure_session
    require_command jq
    require_command git
    load_approved_deployment_inputs
    if [ "${AIRWIKI_BETA_REPLACE_CONFIRM:-}" != "replace-${side}-beta-node" ]; then
        echo "explicit confirmation for the selected beta-node replacement is required" >&2
        exit 1
    fi
    east_state="$(group_state "${east_group}")"
    west_state="$(group_state "${west_group}")"
    case "${side}" in
        east)
            target_state="${east_state}"
            healthy_state="${west_state}"
            healthy_group="${west_group}"
            healthy_node="${west_node}"
            ;;
        west)
            target_state="${west_state}"
            healthy_state="${east_state}"
            healthy_group="${east_group}"
            healthy_node="${east_node}"
            ;;
    esac
    if [ "${healthy_state}" != "beta" ] ||
        { [ "${target_state}" != "beta" ] && [ "${target_state}" != "absent" ]; }; then
        echo "replacement requires one healthy beta node and a beta or absent target" >&2
        exit 1
    fi
    if ! node_service_is_active "${healthy_group}" "${healthy_node}"; then
        echo "replacement requires the retained beta node VM and service to be active" >&2
        exit 1
    fi

    "${cost_script}" --check
    if [ "${target_state}" = "beta" ]; then
        az group delete \
            --name "${group}" \
            --yes \
            --output none \
            --only-show-errors
    fi
    if [ "$(group_state "${group}")" != "absent" ]; then
        echo "the selected old beta-node resource group was not removed" >&2
        exit 1
    fi

    budget_body="$(mktemp "${TMPDIR:-/tmp}/airwiki-beta-budget.XXXXXX")"
    deploy_started=1
    deploy_succeeded=0
    rollback_east=0
    rollback_west=0
    case "${side}" in
        east) rollback_east=1 ;;
        west) rollback_west=1 ;;
    esac
    trap cleanup_failed_deploy 0
    trap 'exit 1' HUP INT TERM
    provision_node_group "${group}" "${region}" "${node}"
    deploy_succeeded=1
    trap - 0 HUP INT TERM
    rm -f -- "${budget_body}"
    budget_body=
    printf '%s\n' \
        "${side} beta node replaced with a fresh persistent disk and identity slot" \
        "install the approved candidate on this node, then build a higher bootstrap registry"
}

require_private_key() {
    private_key="${AIRWIKI_BETA_SSH_PRIVATE_KEY_FILE:-}"
    if [ -z "${private_key}" ] || [ -L "${private_key}" ] || [ ! -f "${private_key}" ]; then
        echo "AIRWIKI_BETA_SSH_PRIVATE_KEY_FILE must be a regular non-symlink file" >&2
        exit 1
    fi
    printf '%s\n' "${private_key}"
}

install_node() {
    group="$1"
    node="$2"
    release_binary="$3"
    expected_sha256="$4"
    private_key="$5"
    working_directory="$6"

    public_ipv4="$(
        az network public-ip show \
            --resource-group "${group}" \
            --name "${node}-pip" \
            --query ipAddress \
            -o tsv
    )"
    expected_host_fingerprint="$(
        az vm run-command invoke \
            --resource-group "${group}" \
            --name "${node}" \
            --command-id RunShellScript \
            --scripts "ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub -E sha256 | awk '{print \$2}'" \
            --query "value[0].message" \
            -o tsv |
            sed -n 's/^SHA256:[A-Za-z0-9+/=]*$/&/p' |
            head -n 1
    )"
    if [ -z "${expected_host_fingerprint}" ]; then
        echo "could not obtain the Azure-attested SSH host-key fingerprint" >&2
        exit 1
    fi
    known_hosts="${working_directory}/${node}.known_hosts"
    if ! ssh-keyscan -T 10 -t ed25519 "${public_ipv4}" >"${known_hosts}" 2>/dev/null; then
        echo "could not scan the beta node SSH host key" >&2
        exit 1
    fi
    scanned_host_fingerprint="$(
        ssh-keygen -lf "${known_hosts}" -E sha256 |
            awk '{print $2}' |
            head -n 1
    )"
    if [ "${scanned_host_fingerprint}" != "${expected_host_fingerprint}" ]; then
        echo "the beta node SSH host key does not match Azure control-plane evidence" >&2
        exit 1
    fi
    scp \
        -i "${private_key}" \
        -o BatchMode=yes \
        -o IdentitiesOnly=yes \
        -o StrictHostKeyChecking=yes \
        -o "UserKnownHostsFile=${known_hosts}" \
        -o LogLevel=ERROR \
        "${release_binary}" \
        "${admin_username}@${public_ipv4}:/tmp/airwiki-federation-index.candidate"
    scp \
        -i "${private_key}" \
        -o BatchMode=yes \
        -o IdentitiesOnly=yes \
        -o StrictHostKeyChecking=yes \
        -o "UserKnownHostsFile=${known_hosts}" \
        -o LogLevel=ERROR \
        "${install_script}" \
        "${admin_username}@${public_ipv4}:/tmp/azure-install.sh"
    ssh \
        -i "${private_key}" \
        -o BatchMode=yes \
        -o IdentitiesOnly=yes \
        -o StrictHostKeyChecking=yes \
        -o "UserKnownHostsFile=${known_hosts}" \
        -o LogLevel=ERROR \
        "${admin_username}@${public_ipv4}" \
        "chmod 0755 /tmp/airwiki-federation-index.candidate /tmp/azure-install.sh && sudo AIRWIKI_FEDERATION_BINARY_SHA256='${expected_sha256}' /tmp/azure-install.sh /tmp/airwiki-federation-index.candidate '${public_ipv4}' 1 && sudo systemctl is-active --quiet airwiki-federation-index-1.service && rm -f /tmp/airwiki-federation-index.candidate /tmp/azure-install.sh"
}

install_candidate() {
    [ "$#" -ge 1 ] && [ "$#" -le 2 ] || usage
    require_azure_session
    require_command file
    require_command ssh
    require_command scp
    require_command ssh-keyscan
    require_command ssh-keygen
    release_binary="$1"
    install_side="${2:-both}"
    case "${install_side}" in
        both) require_beta_groups ;;
        east) require_beta_group "${east_group}" ;;
        west) require_beta_group "${west_group}" ;;
        *) usage ;;
    esac
    if [ -L "${release_binary}" ] || [ ! -f "${release_binary}" ] || [ ! -x "${release_binary}" ]; then
        echo "the release binary must be an executable regular non-symlink file" >&2
        exit 1
    fi
    case "$(LC_ALL=C file -b -- "${release_binary}")" in
        "ELF 64-bit LSB executable, x86-64,"* | "ELF 64-bit LSB pie executable, x86-64,"*) ;;
        *)
            echo "the release binary must be an x86-64 Linux ELF candidate" >&2
            exit 1
            ;;
    esac
    private_key="$(require_private_key)"
    if command -v shasum >/dev/null 2>&1; then
        expected_sha256="$(shasum -a 256 -- "${release_binary}" | awk '{print $1}')"
    else
        expected_sha256="$(sha256sum -- "${release_binary}" | awk '{print $1}')"
    fi
    install_working_directory="$(mktemp -d "${TMPDIR:-/tmp}/airwiki-beta-install.XXXXXX")"
    trap 'rm -rf -- "${install_working_directory}"' EXIT HUP INT TERM
    if [ "${install_side}" = "both" ] || [ "${install_side}" = "east" ]; then
        install_node \
            "${east_group}" "${east_node}" "${release_binary}" \
            "${expected_sha256}" "${private_key}" "${install_working_directory}"
    fi
    if [ "${install_side}" = "both" ] || [ "${install_side}" = "west" ]; then
        install_node \
            "${west_group}" "${west_node}" "${release_binary}" \
            "${expected_sha256}" "${private_key}" "${install_working_directory}"
    fi
    rm -rf -- "${install_working_directory}"
    trap - EXIT HUP INT TERM
    printf '%s\n' "the exact release binary is active on the selected ${install_side} beta scope"
}

rfc3339_epoch() {
    if date -u -d "$1" +%s >/dev/null 2>&1; then
        date -u -d "$1" +%s
    elif date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "$1" +%s >/dev/null 2>&1; then
        date -u -j -f "%Y-%m-%dT%H:%M:%SZ" "$1" +%s
    else
        return 1
    fi
}

file_permissions() {
    if permissions="$(stat -f '%Lp' "$1" 2>/dev/null)"; then
        printf '%s\n' "${permissions}"
    else
        stat -c '%a' "$1"
    fi
}

peer_id_for_node() {
    group="$1"
    node="$2"
    message="$(
        az vm run-command invoke \
            --resource-group "${group}" \
            --name "${node}" \
            --command-id RunShellScript \
            --scripts "sudo -u airwiki /usr/local/bin/airwiki-federation-index /var/lib/airwiki-federation/index-1/index.db --print-peer-id" \
            --query "value[0].message" \
            -o tsv
    )"
    peer_id="$(printf '%s\n' "${message}" | sed -n 's/^\(12D3Koo[A-Za-z0-9]*\)$/\1/p' | head -n 1)"
    if [ -z "${peer_id}" ]; then
        echo "could not obtain one beta-node identity" >&2
        exit 1
    fi
    printf '%s\n' "${peer_id}"
}

create_bootstrap() {
    [ "$#" -ge 2 ] && [ "$#" -le 3 ] || usage
    require_azure_session
    require_command git
    registry_version="$1"
    expiry="$2"
    registry_mode="${3:-both}"
    case "${registry_version}" in
        '' | *[!0-9]* | 0) usage ;;
    esac
    case "${registry_mode}" in
        both | without-east | without-west) ;;
        expired) expiry="2020-01-01T00:00:00Z" ;;
        *) usage ;;
    esac
    if [ "${registry_mode}" != "expired" ]; then
        expiry_epoch="$(rfc3339_epoch "${expiry}")" || {
            echo "expiry must use UTC RFC 3339 seconds, for example 2026-10-31T23:59:59Z" >&2
            exit 1
        }
        now_epoch="$(date -u +%s)"
        min_epoch=$((now_epoch + 30 * 24 * 60 * 60))
        max_epoch=$((now_epoch + 120 * 24 * 60 * 60))
        if [ "${expiry_epoch}" -lt "${min_epoch}" ] || [ "${expiry_epoch}" -gt "${max_epoch}" ]; then
            echo "bootstrap expiry must be between 30 and 120 days from now" >&2
            exit 1
        fi
    fi

    east_peer=
    west_peer=
    east_ipv4=
    west_ipv4=
    case "${registry_mode}" in
        both | expired)
            require_beta_groups
            east_peer="$(peer_id_for_node "${east_group}" "${east_node}")"
            west_peer="$(peer_id_for_node "${west_group}" "${west_node}")"
            if [ "${east_peer}" = "${west_peer}" ]; then
                echo "the two beta nodes do not have independent identities" >&2
                exit 1
            fi
            east_ipv4="$(
                az network public-ip show \
                    --resource-group "${east_group}" \
                    --name "${east_node}-pip" \
                    --query ipAddress \
                    -o tsv
            )"
            west_ipv4="$(
                az network public-ip show \
                    --resource-group "${west_group}" \
                    --name "${west_node}-pip" \
                    --query ipAddress \
                    -o tsv
            )"
            ;;
        without-east)
            require_beta_group "${west_group}"
            west_peer="$(peer_id_for_node "${west_group}" "${west_node}")"
            west_ipv4="$(
                az network public-ip show \
                    --resource-group "${west_group}" \
                    --name "${west_node}-pip" \
                    --query ipAddress \
                    -o tsv
            )"
            ;;
        without-west)
            require_beta_group "${east_group}"
            east_peer="$(peer_id_for_node "${east_group}" "${east_node}")"
            east_ipv4="$(
                az network public-ip show \
                    --resource-group "${east_group}" \
                    --name "${east_node}-pip" \
                    --query ipAddress \
                    -o tsv
            )"
            ;;
    esac
    private_directory="${repository_root}/target/private"
    bootstrap_file="${private_directory}/federation-beta-v1.bootstrap"
    umask 077
    install -d -m 0700 "${private_directory}"
    case "${registry_mode}" in
        both | expired)
            printf '%s|%s|%s|/ip4/%s/tcp/42042;%s|%s|%s|/ip4/%s/tcp/42042\n' \
                "${registry_version}" "${expiry}" "${east_peer}" "${east_ipv4}" \
                "${registry_version}" "${expiry}" "${west_peer}" "${west_ipv4}" \
                >"${bootstrap_file}"
            ;;
        without-east)
            printf '%s|%s|%s|/ip4/%s/tcp/42042\n' \
                "${registry_version}" "${expiry}" "${west_peer}" "${west_ipv4}" \
                >"${bootstrap_file}"
            ;;
        without-west)
            printf '%s|%s|%s|/ip4/%s/tcp/42042\n' \
                "${registry_version}" "${expiry}" "${east_peer}" "${east_ipv4}" \
                >"${bootstrap_file}"
            ;;
    esac
    chmod 0600 "${bootstrap_file}"
    if command -v shasum >/dev/null 2>&1; then
        bootstrap_sha256="$(shasum -a 256 -- "${bootstrap_file}" | awk '{print $1}')"
    else
        bootstrap_sha256="$(sha256sum -- "${bootstrap_file}" | awk '{print $1}')"
    fi
    printf '%s\n' \
        "private bootstrap created for mode ${registry_mode}" \
        "registry version: ${registry_version}" \
        "registry expiry: ${expiry}" \
        "registry SHA-256: ${bootstrap_sha256}"
}

sanitized_remote_status() {
    group="$1"
    node="$2"
    status_script='
unit=airwiki-federation-index-1.service
state="$(systemctl is-active "${unit}" 2>/dev/null || true)"
restarts="$(systemctl show "${unit}" --property=NRestarts --value 2>/dev/null || true)"
memory="$(systemctl show "${unit}" --property=MemoryCurrent --value 2>/dev/null || true)"
case "${state}" in
    active | inactive | failed | activating | deactivating) ;;
    *) state=unknown ;;
esac
case "${restarts}" in
    "" | *[!0-9]*) restarts=unknown ;;
esac
case "${memory}" in
    "" | *[!0-9]*) memory=unknown ;;
esac
printf "service_state=%s\nrestart_count=%s\nmemory_current_bytes=%s\n" "${state}" "${restarts}" "${memory}"
journalctl --unit "${unit}" --since "-24 hours" --output cat --no-pager 2>/dev/null |
    sed -n "s/.*error_kind=\"\{0,1\}\([a-z0-9_-]\{1,64\}\).*/\1/p" |
    sort |
    uniq -c |
    awk "{ if (\$2 ~ /^[a-z0-9_-]+$/) printf \"error_class=%s count=%s\\n\", \$2, \$1 }"
'
    response="$(
        az vm run-command invoke \
            --resource-group "${group}" \
            --name "${node}" \
            --command-id RunShellScript \
            --scripts "${status_script}" \
            -o json
    )"
    message="$(
        printf '%s\n' "${response}" |
            jq -er '.value[0].message | select(type == "string" and length > 0)'
    )"
    sanitized_status="$(
        printf '%s\n' "${message}" |
            tr -d '\r' |
            awk '
            /^service_state=(active|inactive|failed|activating|deactivating|unknown)$/ {
                service_state += 1
                if (service_state == 1) print
                next
            }
            /^restart_count=([0-9]+|unknown)$/ {
                restart_count += 1
                if (restart_count == 1) print
                next
            }
            /^memory_current_bytes=([0-9]+|unknown)$/ {
                memory_current_bytes += 1
                if (memory_current_bytes == 1) print
                next
            }
            /^error_class=[a-z0-9_-]+ count=[0-9]+$/ {
                print
                next
            }
            END {
                if (service_state != 1 || restart_count != 1 ||
                    memory_current_bytes != 1) {
                    exit 1
                }
            }
        '
    )"
    printf '%s\n' "${sanitized_status}"
}

status_node() {
    label="$1"
    group="$2"
    node="$3"
    state="$(group_state "${group}")"
    case "${state}" in
        absent)
            printf '%s\n' "${label}: absent"
            return
            ;;
        foreign)
            printf '%s\n' "${label}: unmanaged"
            return
            ;;
        beta) ;;
    esac
    power_state="$(
        az vm get-instance-view \
            --resource-group "${group}" \
            --name "${node}" \
            --query "instanceView.statuses[?starts_with(code, 'PowerState/')].code | [0]" \
            -o tsv
    )"
    vm_id="$(az vm show --resource-group "${group}" --name "${node}" --query id -o tsv)"
    availability="$(
        az monitor metrics list \
            --resource "${vm_id}" \
            --metric VmAvailabilityMetric \
            --interval PT1M \
            --aggregation Minimum \
            --query "value[0].timeseries[0].data[-1].minimum" \
            -o tsv 2>/dev/null || true
    )"
    subscription_id="$(az account show --query id -o tsv)"
    scope="/subscriptions/${subscription_id}/resourceGroups/${group}"
    budget="$(
        az rest \
            --method get \
            --uri "https://management.azure.com${scope}/providers/Microsoft.Consumption/budgets/${budget_name}?api-version=${budget_api_version}" \
            -o json 2>/dev/null || true
    )"
    spend=pending
    if [ -n "${budget}" ]; then
        budget_currency="$(printf '%s\n' "${budget}" | jq -r '.properties.currentSpend.unit // empty')"
        if [ "${budget_currency}" = "USD" ]; then
            candidate_spend="$(
                printf '%s\n' "${budget}" |
                    jq -r '.properties.currentSpend.amount // empty'
            )"
            case "${candidate_spend}" in
                '' | *[!0-9.]* | *.*.*) ;;
                *) spend="${candidate_spend}" ;;
            esac
        else
            spend=currency-mismatch
        fi
    fi
    printf '%s\n' \
        "${label}:" \
        "power_state=${power_state:-unknown}" \
        "availability_metric=${availability:-unknown}" \
        "month_to_date_cost_usd=${spend:-pending}"
    if [ "${power_state}" = "PowerState/running" ]; then
        sanitized_remote_status "${group}" "${node}"
    else
        printf '%s\n' \
            "service_state=unknown" \
            "restart_count=unknown" \
            "memory_current_bytes=unknown"
    fi
}

status_all() {
    [ "$#" -eq 0 ] || usage
    require_azure_session
    require_command jq
    status_node east "${east_group}" "${east_node}"
    status_node west "${west_group}" "${west_node}"
}

set_node_power() {
    action="$1"
    side="$2"
    values="$(node_values "${side}")"
    group="$(printf '%s\n' "${values}" | sed -n '1p')"
    node="$(printf '%s\n' "${values}" | sed -n '3p')"
    require_azure_session
    state="$(group_state "${group}")"
    if [ "${state}" != "beta" ]; then
        echo "the selected dedicated beta resource group is absent or untagged" >&2
        exit 1
    fi
    if [ "${action}" = "stop" ]; then
        az vm deallocate \
            --resource-group "${group}" \
            --name "${node}" \
            --output none \
            --only-show-errors
        printf '%s\n' "${side} beta node deallocated for the failover gate"
    else
        az vm start \
            --resource-group "${group}" \
            --name "${node}" \
            --output none \
            --only-show-errors
        printf '%s\n' "${side} beta node started for recovery"
    fi
}

retire() {
    [ "$#" -eq 0 ] || usage
    require_azure_session
    if [ "${AIRWIKI_BETA_RETIRE_CONFIRM:-}" != "delete-airwiki-federation-beta-v1" ]; then
        echo "explicit beta retirement confirmation is required" >&2
        exit 1
    fi
    retired_version="${AIRWIKI_BETA_BOOTSTRAP_RETIRED_VERSION:-}"
    case "${retired_version}" in
        '' | *[!0-9]* | 0)
            echo "record the higher bootstrap version that revoked these nodes before retirement" >&2
            exit 1
            ;;
    esac
    retired_bootstrap="${repository_root}/target/private/federation-beta-v1.bootstrap"
    if [ -L "${retired_bootstrap}" ] || [ ! -f "${retired_bootstrap}" ]; then
        echo "the private higher-version retirement bootstrap is required" >&2
        exit 1
    fi
    if [ "$(file_permissions "${retired_bootstrap}")" != "600" ] ||
        [ "$(wc -l <"${retired_bootstrap}" | tr -d ' ')" -ne 1 ] ||
        [ "$(wc -c <"${retired_bootstrap}" | tr -d ' ')" -gt 8192 ]; then
        echo "the private retirement bootstrap must be one bounded mode-0600 line" >&2
        exit 1
    fi
    retired_registry="$(sed -n '1p' "${retired_bootstrap}")"
    retired_entry_count="$(printf '%s\n' "${retired_registry}" | awk -F';' '{print NF}')"
    if [ "${retired_entry_count}" -lt 1 ] || [ "${retired_entry_count}" -gt 2 ]; then
        echo "the private retirement bootstrap must contain one or two entries" >&2
        exit 1
    fi
    retired_now_epoch="$(date -u +%s)"
    retired_index=1
    while [ "${retired_index}" -le "${retired_entry_count}" ]; do
        retired_entry="$(
            printf '%s\n' "${retired_registry}" |
                cut -d ';' -f "${retired_index}"
        )"
        if [ "$(printf '%s\n' "${retired_entry}" | awk -F'|' '{print NF}')" -ne 4 ]; then
            echo "one retirement bootstrap entry is malformed" >&2
            exit 1
        fi
        retired_file_version="${retired_entry%%|*}"
        retired_remainder="${retired_entry#*|}"
        retired_expiry="${retired_remainder%%|*}"
        retired_remainder="${retired_remainder#*|}"
        retired_peer="${retired_remainder%%|*}"
        retired_address="${retired_remainder#*|}"
        retired_expiry_epoch="$(rfc3339_epoch "${retired_expiry}")" || {
            echo "one retirement bootstrap expiry is invalid" >&2
            exit 1
        }
        if [ "${retired_file_version}" != "${retired_version}" ] ||
            [ "${retired_expiry_epoch}" -ge "${retired_now_epoch}" ] ||
            [ -z "${retired_peer}" ] ||
            [ -z "${retired_address}" ]; then
            echo "retirement requires an entirely expired matching higher-version registry" >&2
            exit 1
        fi
        retired_index=$((retired_index + 1))
    done

    east_state="$(group_state "${east_group}")"
    west_state="$(group_state "${west_group}")"
    if [ "${east_state}" = "foreign" ] || [ "${west_state}" = "foreign" ]; then
        echo "refusing retirement because a target resource group is untagged" >&2
        exit 1
    fi
    retirement_failed=0
    if [ "${east_state}" = "beta" ] &&
        ! az group delete \
            --name "${east_group}" \
            --yes \
            --output none \
            --only-show-errors; then
        retirement_failed=1
    fi
    if [ "${west_state}" = "beta" ] &&
        ! az group delete \
            --name "${west_group}" \
            --yes \
            --output none \
            --only-show-errors; then
        retirement_failed=1
    fi
    east_state="$(group_state "${east_group}")" || east_state=unknown
    west_state="$(group_state "${west_group}")" || west_state=unknown
    if [ "${east_state}" != "absent" ] || [ "${west_state}" != "absent" ]; then
        retirement_failed=1
    fi
    if [ "${retirement_failed}" -ne 0 ]; then
        echo "RETIREMENT INCOMPLETE: a beta resource group may remain billable; run status and retry retirement" >&2
        exit 1
    fi
    printf '%s\n' \
        "both dedicated beta resource groups and their budgets were deleted" \
        "bootstrap retirement version: ${retired_version}"
}

command="${1:-}"
if [ "$#" -gt 0 ]; then
    shift
fi
case "${command}" in
    deploy) [ "$#" -eq 0 ] || usage; deploy ;;
    replace-node) replace_node "$@" ;;
    install) install_candidate "$@" ;;
    bootstrap) create_bootstrap "$@" ;;
    revoke-bootstrap)
        [ "$#" -eq 3 ] || usage
        case "$3" in
            east | west) create_bootstrap "$1" "$2" "without-$3" ;;
            *) usage ;;
        esac
        ;;
    expired-bootstrap)
        [ "$#" -eq 1 ] || usage
        create_bootstrap "$1" "2020-01-01T00:00:00Z" expired
        ;;
    status) status_all "$@" ;;
    stop-node) [ "$#" -eq 1 ] || usage; set_node_power stop "$1" ;;
    start-node) [ "$#" -eq 1 ] || usage; set_node_power start "$1" ;;
    retire) retire "$@" ;;
    *) usage ;;
esac
