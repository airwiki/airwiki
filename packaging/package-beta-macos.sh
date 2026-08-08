#!/bin/sh
set -eu

validate_only=false
if [ "${1:-}" = "--validate-only" ]; then
    validate_only=true
    shift
fi
if [ "$#" -ne 1 ]; then
    echo "usage: package-beta-macos.sh [--validate-only] <private-bootstrap-file>" >&2
    exit 1
fi

readonly bootstrap_file="$1"
readonly expected_sha="${AIRWIKI_BETA_CANDIDATE_SHA:-}"

if [ -z "${expected_sha}" ]; then
    echo "AIRWIKI_BETA_CANDIDATE_SHA is required" >&2
    exit 1
fi
if [ -L "${bootstrap_file}" ] || [ ! -f "${bootstrap_file}" ]; then
    echo "the beta bootstrap must be a regular non-symlink file" >&2
    exit 1
fi
if [ "$(wc -l <"${bootstrap_file}" | tr -d ' ')" -ne 1 ] ||
    [ "$(wc -c <"${bootstrap_file}" | tr -d ' ')" -gt 8192 ]; then
    echo "the beta bootstrap must be one bounded line" >&2
    exit 1
fi
permissions="$(stat -f '%Lp' "${bootstrap_file}")"
if [ "${permissions}" != "600" ]; then
    echo "the private beta bootstrap must have mode 0600" >&2
    exit 1
fi

repository_root="$(git rev-parse --show-toplevel)"
current_sha="$(git -C "${repository_root}" rev-parse HEAD)"
if [ "${current_sha}" != "${expected_sha}" ]; then
    echo "AIRWIKI_BETA_CANDIDATE_SHA does not match HEAD" >&2
    exit 1
fi
if [ -n "$(git -C "${repository_root}" status --porcelain --untracked-files=normal)" ]; then
    echo "beta packaging requires a clean worktree" >&2
    exit 1
fi

bootstrap="$(sed -n '1p' "${bootstrap_file}")"
printf '%s\n' "${bootstrap}" | awk -F';' '
    NF < 1 || NF > 2 { exit 1 }
    {
        east_fields = split($1, east, "|");
        if (east_fields != 4 || east[1] !~ /^[1-9][0-9]*$/) {
            exit 1
        }
        if (NF == 2) {
            west_fields = split($2, west, "|");
            if (west_fields != 4 || east[1] != west[1] ||
                east[2] != west[2] || east[3] == west[3] ||
                east[4] == west[4]) {
                exit 1
            }
        }
    }
' || {
    echo "the private bootstrap is not one coherent one- or two-node registry" >&2
    exit 1
}

if command -v shasum >/dev/null 2>&1; then
    bootstrap_sha256="$(shasum -a 256 -- "${bootstrap_file}" | awk '{print $1}')"
else
    bootstrap_sha256="$(sha256sum -- "${bootstrap_file}" | awk '{print $1}')"
fi
if [ "${validate_only}" = true ]; then
    printf '%s\n' "macOS beta bootstrap packaging policy: PASS"
    exit 0
fi
AIRWIKI_BOOTSTRAP_FEDERATION_INDEXES="${bootstrap}" \
    "${repository_root}/packaging/package-macos.sh"
printf '%s\n' \
    "macOS arm64 beta candidate built from ${current_sha}" \
    "bootstrap SHA-256: ${bootstrap_sha256}"
