#!/usr/bin/env bash
################################################################################
# IBM webMethods B2B Toolkit
#
# A single-file toolkit for IBM webMethods B2B SaaS. Use --feature to select
# the capability you need:
#
#   setup   — Provision B2B assets (partners, channels, users, document types)
#             for a given domain. Idempotent — safe to run multiple times.
#
#   trace   — Provision assets (if needed) and submit a complete multi-document
#             EDI trace sequence for retail, healthcare, or supply-chain demos.
#
# Configuration lives entirely in toolkit.env — never edit this script.
################################################################################

set -euo pipefail

################################################################################
# LOAD ENVIRONMENT CONFIG
################################################################################

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ENV_FILE="${SCRIPT_DIR}/toolkit.env"


if [[ ! -f "$ENV_FILE" ]]; then
    echo "[ERROR] Configuration file not found: $ENV_FILE" >&2
    echo "        Copy toolkit.env.example to toolkit.env and fill in your values." >&2
    exit 1
fi

set +u
# shellcheck disable=SC1090
source "$ENV_FILE"
set -u

for _required_var in B2B_HOST MCSP_INSTANCE_ID MCSP_API_KEY; do
    if [[ -z "${!_required_var:-}" ]]; then
        echo "[ERROR] Required variable '$_required_var' is not set in $ENV_FILE" >&2
        exit 1
    fi
done
unset _required_var

################################################################################
# RUNTIME CONSTANTS
################################################################################

DEFAULT_TIMEOUT=30
POST_PAYLOAD_TIMEOUT=120

################################################################################
# UTILITY FUNCTIONS
################################################################################

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log() {
    local level="$1"
    local message="$2"
    local color=""
    case "$level" in
        INFO)    color="$BLUE"   ;;
        WARN)    color="$YELLOW" ;;
        ERROR)   color="$RED"    ;;
        SUCCESS) color="$GREEN"  ;;
        *)       color="$NC"     ;;
    esac
    echo -e "${color}[$level]${NC} $message" >&2
}

extract_json_value() {
    local json="$1"
    local path="$2"
    if command -v jq &> /dev/null; then
        echo "$json" | jq -r "$path" 2>/dev/null || echo ""
    else
        log "ERROR" "jq is not installed. Please install jq for JSON parsing."
        return 1
    fi
}

check_dependencies() {
    if [[ "${BASH_VERSINFO[0]}" -lt 4 ]]; then
        log "ERROR" "bash 4 or higher is required (found ${BASH_VERSION})."
        log "ERROR" "On macOS, install a newer bash with: brew install bash"
        log "ERROR" "Then re-run with: /opt/homebrew/bin/bash $0"
        return 1
    fi

    local missing_tools=()
    command -v curl &> /dev/null || missing_tools+=("curl")
    command -v jq   &> /dev/null || missing_tools+=("jq")

    if [[ ${#missing_tools[@]} -gt 0 ]]; then
        log "ERROR" "Missing required tools: ${missing_tools[*]}"
        echo "Installation instructions:"
        echo "  macOS:         brew install ${missing_tools[*]}"
        echo "  Ubuntu/Debian: sudo apt-get install ${missing_tools[*]}"
        echo "  RHEL/CentOS:   sudo yum install ${missing_tools[*]}"
        return 1
    fi

    log "SUCCESS" "All required dependencies are installed"
    return 0
}

display_banner() {
    echo -e "${BLUE}"
    echo "╔════════════════════════════════════════════════════════════╗"
    echo "║           IBM webMethods B2B Toolkit                       ║"
    echo "║      Multi-Feature EDI & B2B Asset Management Tool        ║"
    echo "╚════════════════════════════════════════════════════════════╝"
    echo -e "${NC}"
    log "INFO" "Host:     $B2B_HOST"
    log "INFO" "Instance: $MCSP_INSTANCE_ID"
}

################################################################################
# WORKSPACE STATE MANAGEMENT
################################################################################

WORKSPACE_ROOT=".b2b-toolkit"

WS_PARTNER1_INTERNAL_ID=""
WS_PARTNER2_INTERNAL_ID=""
WS_CHANNEL_ID=""
WS_CHANNEL_ENDPOINT_URI=""
WS_PARTNER_USER_ID=""
WS_P2_CHANNEL_ID=""
WS_P2_CHANNEL_ENDPOINT_URI=""
WS_P2_PARTNER_USER_ID=""
WS_RUN_COUNT=0
WS_ISA_CTRL_NUM=1
WS_GS_CTRL_NUM=1

resolve_workspace_path() {
    local domain="$1"
    local sanitized_host
    sanitized_host=$(echo "$B2B_HOST" | tr '.' '-' | tr '[:upper:]' '[:lower:]')
    WORKSPACE_DIR="${WORKSPACE_ROOT}/${sanitized_host}"
    WORKSPACE_FILE="${WORKSPACE_DIR}/${domain}.json"
    log "INFO" "Workspace directory : $WORKSPACE_DIR"
    log "INFO" "Workspace file      : $WORKSPACE_FILE"
}

load_workspace() {
    local domain="$1"

    if [[ -f "$WORKSPACE_FILE" ]]; then
        log "INFO" "Loading workspace from $WORKSPACE_FILE"

        WS_PARTNER1_INTERNAL_ID=$(   jq -r '.assets.partner1_internal_id    // ""' "$WORKSPACE_FILE")
        WS_PARTNER2_INTERNAL_ID=$(   jq -r '.assets.partner2_internal_id    // ""' "$WORKSPACE_FILE")
        WS_CHANNEL_ID=$(             jq -r '.assets.channel_id              // ""' "$WORKSPACE_FILE")
        WS_CHANNEL_ENDPOINT_URI=$(   jq -r '.assets.channel_endpoint_uri    // ""' "$WORKSPACE_FILE")
        WS_PARTNER_USER_ID=$(        jq -r '.assets.partner_user_id         // ""' "$WORKSPACE_FILE")
        WS_P2_CHANNEL_ID=$(          jq -r '.assets.p2_channel_id           // ""' "$WORKSPACE_FILE")
        WS_P2_CHANNEL_ENDPOINT_URI=$(jq -r '.assets.p2_channel_endpoint_uri // ""' "$WORKSPACE_FILE")
        WS_P2_PARTNER_USER_ID=$(     jq -r '.assets.p2_partner_user_id      // ""' "$WORKSPACE_FILE")

        WS_RUN_COUNT=$(    jq -r '.counters.run_count          // 0' "$WORKSPACE_FILE")
        WS_ISA_CTRL_NUM=$( jq -r '.counters.isa_control_number // 1' "$WORKSPACE_FILE")
        WS_GS_CTRL_NUM=$(  jq -r '.counters.gs_control_number  // 1' "$WORKSPACE_FILE")
        [[ "$WS_ISA_CTRL_NUM" -lt 1 ]] && WS_ISA_CTRL_NUM=1
        [[ "$WS_GS_CTRL_NUM"  -lt 1 ]] && WS_GS_CTRL_NUM=1

        declare -gA WS_DOCTYPE_IDS=()
        while IFS=$'\t' read -r key value; do
            [[ -n "$key" && "$key" != "null" ]] && WS_DOCTYPE_IDS["$key"]="$value"
        done < <(jq -r '.assets.document_type_ids // {} | to_entries[] | [.key, .value] | @tsv' "$WORKSPACE_FILE")

        log "SUCCESS" "Workspace loaded for host: $B2B_HOST, domain: $domain"
    else
        WS_PARTNER1_INTERNAL_ID=""
        WS_PARTNER2_INTERNAL_ID=""
        WS_CHANNEL_ID=""
        WS_CHANNEL_ENDPOINT_URI=""
        WS_PARTNER_USER_ID=""
        WS_P2_CHANNEL_ID=""
        WS_P2_CHANNEL_ENDPOINT_URI=""
        WS_P2_PARTNER_USER_ID=""
        WS_RUN_COUNT=0
        WS_ISA_CTRL_NUM=1
        WS_GS_CTRL_NUM=1
        declare -gA WS_DOCTYPE_IDS=()
        log "INFO" "No workspace found — fresh start for host: $B2B_HOST, domain: $domain"
    fi
}

save_workspace() {
    local domain="$1"
    mkdir -p "$WORKSPACE_DIR"

    local created_at
    if [[ -f "$WORKSPACE_FILE" ]]; then
        created_at=$(jq -r '.meta.created_at // ""' "$WORKSPACE_FILE")
    fi
    if [[ -z "${created_at:-}" || "$created_at" == "null" ]]; then
        created_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')
    fi

    local last_run
    last_run=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

    local doctype_json="{}"
    if [[ ${#WS_DOCTYPE_IDS[@]} -gt 0 ]]; then
        local kv_args=()
        for key in "${!WS_DOCTYPE_IDS[@]}"; do
            kv_args+=("$key" "${WS_DOCTYPE_IDS[$key]}")
        done
        doctype_json=$(jq -n '
            [ $ARGS.positional | _nwise(2) | { (.[0]): .[1] } ] | add // {}
        ' --args "${kv_args[@]}")
    fi

    local tmp_file
    tmp_file=$(mktemp "${WORKSPACE_DIR}/.ws_tmp_XXXXXX")

    jq -n \
        --arg     hostname             "$B2B_HOST"                    \
        --arg     mcsp_instance_id     "$MCSP_INSTANCE_ID"            \
        --arg     domain               "$domain"                      \
        --arg     created_at           "$created_at"                  \
        --arg     partner1_id          "$WS_PARTNER1_INTERNAL_ID"     \
        --arg     partner2_id          "$WS_PARTNER2_INTERNAL_ID"     \
        --arg     channel_id           "$WS_CHANNEL_ID"               \
        --arg     channel_uri          "$WS_CHANNEL_ENDPOINT_URI"     \
        --arg     partner_user_id      "$WS_PARTNER_USER_ID"          \
        --arg     p2_channel_id        "$WS_P2_CHANNEL_ID"            \
        --arg     p2_channel_uri       "$WS_P2_CHANNEL_ENDPOINT_URI"  \
        --arg     p2_partner_user_id   "$WS_P2_PARTNER_USER_ID"       \
        --argjson doctype_ids          "$doctype_json"                \
        --argjson isa_ctrl             "${WS_ISA_CTRL_NUM}"           \
        --argjson gs_ctrl              "${WS_GS_CTRL_NUM}"            \
        --argjson run_count            "${WS_RUN_COUNT}"              \
        --arg     last_run             "$last_run"                    \
    '{
        "meta": {
            "hostname":         $hostname,
            "mcsp_instance_id": $mcsp_instance_id,
            "domain":           $domain,
            "created_at":       $created_at
        },
        "assets": {
            "partner1_internal_id":    $partner1_id,
            "partner2_internal_id":    $partner2_id,
            "channel_id":              $channel_id,
            "channel_endpoint_uri":    $channel_uri,
            "partner_user_id":         $partner_user_id,
            "p2_channel_id":           $p2_channel_id,
            "p2_channel_endpoint_uri": $p2_channel_uri,
            "p2_partner_user_id":      $p2_partner_user_id,
            "document_type_ids":       $doctype_ids
        },
        "counters": {
            "isa_control_number": $isa_ctrl,
            "gs_control_number":  $gs_ctrl,
            "run_count":          $run_count
        },
        "last_run": $last_run
    }' > "$tmp_file"

    mv "$tmp_file" "$WORKSPACE_FILE"
    log "SUCCESS" "Workspace saved: $WORKSPACE_FILE"
}

is_asset_provisioned() {
    if [[ -n "$WS_PARTNER1_INTERNAL_ID"    && \
          -n "$WS_PARTNER2_INTERNAL_ID"    && \
          -n "$WS_CHANNEL_ID"              && \
          -n "$WS_CHANNEL_ENDPOINT_URI"    && \
          -n "$WS_PARTNER_USER_ID"         && \
          -n "$WS_P2_CHANNEL_ID"           && \
          -n "$WS_P2_CHANNEL_ENDPOINT_URI" && \
          -n "$WS_P2_PARTNER_USER_ID" ]]; then
        return 0
    fi
    return 1
}

print_workspace_status() {
    local domain="$1"
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Workspace Status"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    printf "  %-26s %s\n" "Host:"           "$B2B_HOST"
    printf "  %-26s %s\n" "Domain:"         "$domain"
    printf "  %-26s %s\n" "Workspace file:" "$WORKSPACE_FILE"
    echo ""
    printf "  %-30s %s\n" "Partner 1 ID:"       "${WS_PARTNER1_INTERNAL_ID:-(not provisioned)}"
    printf "  %-30s %s\n" "Partner 2 ID:"       "${WS_PARTNER2_INTERNAL_ID:-(not provisioned)}"
    printf "  %-30s %s\n" "P1 Channel ID:"      "${WS_CHANNEL_ID:-(not provisioned)}"
    printf "  %-30s %s\n" "P1 Channel URI:"     "${WS_CHANNEL_ENDPOINT_URI:-(not provisioned)}"
    printf "  %-30s %s\n" "P1 Partner User ID:" "${WS_PARTNER_USER_ID:-(not provisioned)}"
    printf "  %-30s %s\n" "P2 Channel ID:"      "${WS_P2_CHANNEL_ID:-(not provisioned)}"
    printf "  %-30s %s\n" "P2 Channel URI:"     "${WS_P2_CHANNEL_ENDPOINT_URI:-(not provisioned)}"
    printf "  %-30s %s\n" "P2 Partner User ID:" "${WS_P2_PARTNER_USER_ID:-(not provisioned)}"
    echo ""
    if [[ ${#WS_DOCTYPE_IDS[@]} -gt 0 ]]; then
        printf "  %-26s\n" "Document Type IDs:"
        for txn in $(echo "${!WS_DOCTYPE_IDS[@]}" | tr ' ' '\n' | sort); do
            printf "    %-22s %s\n" "${txn}:" "${WS_DOCTYPE_IDS[$txn]}"
        done
    else
        printf "  %-26s %s\n" "Document Type IDs:" "(none)"
    fi
    echo ""
    printf "  %-26s %s\n" "Run count:"          "$WS_RUN_COUNT"
    printf "  %-26s %s\n" "ISA control number:" "$WS_ISA_CTRL_NUM"
    printf "  %-26s %s\n" "GS control number:"  "$WS_GS_CTRL_NUM"
    if [[ -f "$WORKSPACE_FILE" ]]; then
        local last_run
        last_run=$(jq -r '.last_run // "(never)"' "$WORKSPACE_FILE")
        printf "  %-26s %s\n" "Last run:" "$last_run"
    fi
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
}

################################################################################
# DOMAIN CONFIGURATION
################################################################################

DOMAIN_DOC_SEQUENCE=()
DOMAIN_TERMINATOR=""
DOMAIN_ISA_VERSION=""
DOMAIN_P1_NAME=""
DOMAIN_P1_ORG_UNIT=""
DOMAIN_P1_DUNS=""
DOMAIN_P1_USERNAME=""
DOMAIN_P1_PASSWORD=""
DOMAIN_P1_FULLNAME=""
DOMAIN_P2_NAME=""
DOMAIN_P2_ORG_UNIT=""
DOMAIN_P2_DUNS=""
DOMAIN_P2_USERNAME=""
DOMAIN_P2_PASSWORD=""
DOMAIN_P2_FULLNAME=""
DOMAIN_CHANNEL_NAME=""
DOMAIN_P2_CHANNEL_NAME=""

load_domain_config() {
    local domain="$1"
    case "$domain" in
        retail)
            DOMAIN_TERMINATOR="~"
            DOMAIN_ISA_VERSION="00401"
            DOMAIN_P1_NAME="${RETAIL_P1_NAME}"
            DOMAIN_P1_ORG_UNIT="${RETAIL_P1_ORG_UNIT}"
            DOMAIN_P1_DUNS="${RETAIL_P1_DUNS}"
            DOMAIN_P1_USERNAME="${RETAIL_P1_USERNAME}"
            DOMAIN_P1_PASSWORD="${RETAIL_P1_PASSWORD}"
            DOMAIN_P1_FULLNAME="${RETAIL_P1_FULLNAME}"
            DOMAIN_P2_NAME="${RETAIL_P2_NAME}"
            DOMAIN_P2_ORG_UNIT="${RETAIL_P2_ORG_UNIT}"
            DOMAIN_P2_DUNS="${RETAIL_P2_DUNS}"
            DOMAIN_P2_USERNAME="${RETAIL_P2_USERNAME}"
            DOMAIN_P2_PASSWORD="${RETAIL_P2_PASSWORD}"
            DOMAIN_P2_FULLNAME="${RETAIL_P2_FULLNAME}"
            DOMAIN_CHANNEL_NAME="${RETAIL_CHANNEL_NAME}"
            DOMAIN_P2_CHANNEL_NAME="${RETAIL_P2_CHANNEL_NAME}"
            DOMAIN_DOC_SEQUENCE=(840 843 850 855 860 865 856 810 820)
            ;;
        healthcare)
            DOMAIN_TERMINATOR="!"
            DOMAIN_ISA_VERSION="00200"
            DOMAIN_P1_NAME="${HEALTHCARE_P1_NAME}"
            DOMAIN_P1_ORG_UNIT="${HEALTHCARE_P1_ORG_UNIT}"
            DOMAIN_P1_DUNS="${HEALTHCARE_P1_DUNS}"
            DOMAIN_P1_USERNAME="${HEALTHCARE_P1_USERNAME}"
            DOMAIN_P1_PASSWORD="${HEALTHCARE_P1_PASSWORD}"
            DOMAIN_P1_FULLNAME="${HEALTHCARE_P1_FULLNAME}"
            DOMAIN_P2_NAME="${HEALTHCARE_P2_NAME}"
            DOMAIN_P2_ORG_UNIT="${HEALTHCARE_P2_ORG_UNIT}"
            DOMAIN_P2_DUNS="${HEALTHCARE_P2_DUNS}"
            DOMAIN_P2_USERNAME="${HEALTHCARE_P2_USERNAME}"
            DOMAIN_P2_PASSWORD="${HEALTHCARE_P2_PASSWORD}"
            DOMAIN_P2_FULLNAME="${HEALTHCARE_P2_FULLNAME}"
            DOMAIN_CHANNEL_NAME="${HEALTHCARE_CHANNEL_NAME}"
            DOMAIN_P2_CHANNEL_NAME="${HEALTHCARE_P2_CHANNEL_NAME}"
            DOMAIN_DOC_SEQUENCE=(837 275 277 835 820)
            ;;
        supply_chain)
            DOMAIN_TERMINATOR="!"
            DOMAIN_ISA_VERSION="00200"
            DOMAIN_P1_NAME="${SUPPLY_CHAIN_P1_NAME}"
            DOMAIN_P1_ORG_UNIT="${SUPPLY_CHAIN_P1_ORG_UNIT}"
            DOMAIN_P1_DUNS="${SUPPLY_CHAIN_P1_DUNS}"
            DOMAIN_P1_USERNAME="${SUPPLY_CHAIN_P1_USERNAME}"
            DOMAIN_P1_PASSWORD="${SUPPLY_CHAIN_P1_PASSWORD}"
            DOMAIN_P1_FULLNAME="${SUPPLY_CHAIN_P1_FULLNAME}"
            DOMAIN_P2_NAME="${SUPPLY_CHAIN_P2_NAME}"
            DOMAIN_P2_ORG_UNIT="${SUPPLY_CHAIN_P2_ORG_UNIT}"
            DOMAIN_P2_DUNS="${SUPPLY_CHAIN_P2_DUNS}"
            DOMAIN_P2_USERNAME="${SUPPLY_CHAIN_P2_USERNAME}"
            DOMAIN_P2_PASSWORD="${SUPPLY_CHAIN_P2_PASSWORD}"
            DOMAIN_P2_FULLNAME="${SUPPLY_CHAIN_P2_FULLNAME}"
            DOMAIN_CHANNEL_NAME="${SUPPLY_CHAIN_CHANNEL_NAME}"
            DOMAIN_P2_CHANNEL_NAME="${SUPPLY_CHAIN_P2_CHANNEL_NAME}"
            DOMAIN_DOC_SEQUENCE=(204 990 211 214 210 820)
            ;;
        *)
            log "ERROR" "Unknown domain: '$domain'. Valid options: retail | healthcare | supply_chain"
            return 1
            ;;
    esac
    log "SUCCESS" "Domain config loaded: $domain (${#DOMAIN_DOC_SEQUENCE[@]} documents, terminator='${DOMAIN_TERMINATOR}')"
    return 0
}

txn_to_doctype_key() {
    local txn="$1"
    printf "WMX12-4010--%s---------" "$txn"
}

################################################################################
# ASSET PROVISIONING
################################################################################

MCSP_TOKEN=""

acquire_mcsp_token() {
    log "INFO" "Acquiring MCSP token..."

    local token_url="https://account-iam.platform.test.saas.ibm.com/api/2.0/services/${MCSP_INSTANCE_ID}/apikeys/token?includeBuiltinActions=false&includeCustomActions=false&includeRoles=true&prefixRolesWithDefinitionScope=false"

    local response
    response=$(curl -s \
        -X POST \
        -H "Mcsp-ApiKey: ${MCSP_API_KEY}" \
        -H "Content-Type: application/json" \
        --max-time "$DEFAULT_TIMEOUT" \
        "$token_url" 2>&1)

    local curl_exit=$?
    if [[ $curl_exit -ne 0 ]]; then
        log "ERROR" "Token request failed (curl exit $curl_exit): $response"
        return 1
    fi

    MCSP_TOKEN=$(echo "$response" | jq -r '.token // ""' 2>/dev/null)
    if [[ -z "$MCSP_TOKEN" || "$MCSP_TOKEN" == "null" ]]; then
        log "ERROR" "Failed to extract token from response: $response"
        return 1
    fi

    log "SUCCESS" "MCSP token acquired (first 40 chars): ${MCSP_TOKEN:0:40}..."
    return 0
}

create_partner() {
    local corp_name="$1"
    local duns_value="$2"
    local org_unit="$3"

    log "INFO" "Creating partner: $corp_name (DUNS: $duns_value, OrgUnit: $org_unit)"

    local url="https://${B2B_HOST}/b2b/api/v1/partners"
    local body
    body=$(jq -n \
        --arg name "$corp_name" \
        --arg duns "$duns_value" \
        --arg org  "$org_unit" \
        '{
            "corporation": {
                "corporationName": $name,
                "orgUnitName": $org,
                "routingOff": false,
                "status": "Active"
            },
            "identities": [
                { "externalIdType": "DUNS", "externalIdValue": $duns }
            ]
        }')

    local response http_code response_body
    response=$(curl -s -w '\n%{http_code}' \
        -X POST \
        --insecure \
        --max-time "$DEFAULT_TIMEOUT" \
        -H "Authorization: Bearer ${MCSP_TOKEN}" \
        -H "Content-Type: application/json" \
        -d "$body" \
        "$url" 2>&1)

    http_code=$(echo "$response" | tail -n1)
    response_body=$(echo "$response" | sed '$d')

    if [[ "$http_code" -ge 200 && "$http_code" -lt 300 ]]; then
        local internal_id
        internal_id=$(echo "$response_body" | jq -r '.internalId // ""')
        if [[ -z "$internal_id" || "$internal_id" == "null" ]]; then
            log "ERROR" "Partner created but internalId missing. Response: $response_body"
            return 1
        fi
        log "SUCCESS" "Partner '$corp_name' created — internalId: $internal_id"
        echo "$internal_id"
        return 0
    else
        log "ERROR" "Failed to create partner '$corp_name' (HTTP $http_code): $response_body"
        return 1
    fi
}

create_channel() {
    local channel_name="$1"
    local prefix="${2:-p1}"

    log "INFO" "Creating HTTP-IN channel ($prefix): $channel_name"

    local url="https://${B2B_HOST}/b2b/api/v1/channels"
    local body
    body=$(jq -n \
        --arg name "$channel_name" \
        '{
            "channeltype": "HTTP-IN",
            "name": $name,
            "description": "",
            "partnerSpecific": "false",
            "state": "Active",
            "groups": [
                {
                    "name": "flatFile",
                    "fields": [
                        { "name": "keyValuePairs", "value": [] },
                        { "name": "overwrite",     "value": "false" }
                    ],
                    "passwordChange": false
                }
            ]
        }')

    local response http_code response_body
    response=$(curl -s -w '\n%{http_code}' \
        -X POST \
        --insecure \
        --max-time "$DEFAULT_TIMEOUT" \
        -H "Authorization: Bearer ${MCSP_TOKEN}" \
        -H "Content-Type: application/json" \
        -d "$body" \
        "$url" 2>&1)

    http_code=$(echo "$response" | tail -n1)
    response_body=$(echo "$response" | sed '$d')

    if [[ "$http_code" -ge 200 && "$http_code" -lt 300 ]]; then
        local ch_id ch_uri
        ch_id=$(echo "$response_body"  | jq -r '.id           // ""')
        ch_uri=$(echo "$response_body" | jq -r '.endpoint.uri // ""')

        if [[ -z "$ch_id" || "$ch_id" == "null" ]]; then
            log "ERROR" "Channel created but id missing. Response: $response_body"
            return 1
        fi
        if [[ -z "$ch_uri" || "$ch_uri" == "null" ]]; then
            log "ERROR" "Channel created but endpoint.uri missing. Response: $response_body"
            return 1
        fi

        if [[ "$prefix" == "p2" ]]; then
            WS_P2_CHANNEL_ID="$ch_id"
            WS_P2_CHANNEL_ENDPOINT_URI="$ch_uri"
        else
            WS_CHANNEL_ID="$ch_id"
            WS_CHANNEL_ENDPOINT_URI="$ch_uri"
        fi

        log "SUCCESS" "Channel '$channel_name' ($prefix) created — ID: $ch_id, URI: $ch_uri"
        return 0
    else
        log "ERROR" "Failed to create channel '$channel_name' ($prefix) (HTTP $http_code): $response_body"
        return 1
    fi
}

associate_channel_to_partner() {
    local partner_id="$1"
    local channel_id="$2"

    log "INFO" "Associating channel $channel_id to partner $partner_id"

    local url="https://${B2B_HOST}/b2b/api/v1/partners/${partner_id}/b2b-gateways"
    local body
    body=$(jq -n --arg cid "$channel_id" '{"b2bGateways":[{"id": $cid}]}')

    local response http_code response_body
    response=$(curl -s -w '\n%{http_code}' \
        -X PUT \
        --insecure \
        --max-time "$DEFAULT_TIMEOUT" \
        -H "Authorization: Bearer ${MCSP_TOKEN}" \
        -H "Content-Type: application/json" \
        -d "$body" \
        "$url" 2>&1)

    http_code=$(echo "$response" | tail -n1)
    response_body=$(echo "$response" | sed '$d')

    if [[ "$http_code" -ge 200 && "$http_code" -lt 300 ]]; then
        log "SUCCESS" "Channel associated to partner $partner_id"
        return 0
    else
        log "ERROR" "Failed to associate channel (HTTP $http_code): $response_body"
        return 1
    fi
}

create_partner_user() {
    local username="$1"
    local password="$2"
    local fullname="$3"

    log "INFO" "Creating partner user: $username"

    local url="https://${B2B_HOST}/b2b/api/v1/partner-users"
    local body
    body=$(jq -n \
        --arg u "$username" \
        --arg p "$password" \
        --arg f "$fullname" \
        '{"user_name": $u, "password": $p, "full_name": $f}')

    local response http_code response_body
    response=$(curl -s -w '\n%{http_code}' \
        -X POST \
        --insecure \
        --max-time "$DEFAULT_TIMEOUT" \
        -H "Authorization: Bearer ${MCSP_TOKEN}" \
        -H "Content-Type: application/json" \
        -d "$body" \
        "$url" 2>&1)

    http_code=$(echo "$response" | tail -n1)
    response_body=$(echo "$response" | sed '$d')

    if [[ "$http_code" -ge 200 && "$http_code" -lt 300 ]]; then
        local user_id
        user_id=$(echo "$response_body" | jq -r '.id // ""')
        if [[ -z "$user_id" || "$user_id" == "null" ]]; then
            log "ERROR" "Partner user created but id missing. Response: $response_body"
            return 1
        fi
        log "SUCCESS" "Partner user '$username' created — ID: $user_id"
        echo "$user_id"
        return 0
    else
        log "ERROR" "Failed to create partner user '$username' (HTTP $http_code): $response_body"
        return 1
    fi
}

associate_user_to_partner() {
    local partner_id="$1"
    local user_id="$2"
    local username="$3"

    log "INFO" "Associating user $username to partner $partner_id"

    local url="https://${B2B_HOST}/b2b/api/v1/partners/${partner_id}/users"
    local body
    body=$(jq -n \
        --arg uid "$user_id" \
        --arg uname "$username" \
        '{"users":[{"id": $uid, "username": $uname}]}')

    local response http_code response_body
    response=$(curl -s -w '\n%{http_code}' \
        -X PUT \
        --insecure \
        --max-time "$DEFAULT_TIMEOUT" \
        -H "Authorization: Bearer ${MCSP_TOKEN}" \
        -H "Content-Type: application/json" \
        -d "$body" \
        "$url" 2>&1)

    http_code=$(echo "$response" | tail -n1)
    response_body=$(echo "$response" | sed '$d')

    if [[ "$http_code" -ge 200 && "$http_code" -lt 300 ]]; then
        log "SUCCESS" "User $username associated to partner $partner_id"
        return 0
    else
        log "ERROR" "Failed to associate user (HTTP $http_code): $response_body"
        return 1
    fi
}

install_document_type() {
    local txn="$1"
    local doctype_key
    doctype_key=$(txn_to_doctype_key "$txn")

    log "INFO" "Installing document type: $txn ($doctype_key)"

    local url="https://${B2B_HOST}/b2b/api/v1/documents"
    local body
    body=$(jq -n \
        --arg txn "$txn" \
        '{
            "customTransaction": "",
            "referenceDocument": null,
            "standard": "X12",
            "status": "Active",
            "transaction": $txn,
            "type": { "id": "edi", "displayValue": "EDI" },
            "version": "4010",
            "referenceType": "WMINT"
        }')

    local response http_code response_body
    response=$(curl -s -w '\n%{http_code}' \
        -X POST \
        --insecure \
        --max-time "$DEFAULT_TIMEOUT" \
        -H "Authorization: Bearer ${MCSP_TOKEN}" \
        -H "Content-Type: application/json" \
        -d "$body" \
        "$url" 2>&1)

    http_code=$(echo "$response" | tail -n1)
    response_body=$(echo "$response" | sed '$d')

    if [[ "$http_code" -ge 200 && "$http_code" -lt 300 ]]; then
        local doc_id
        doc_id=$(echo "$response_body" | jq -r '.id // ""')
        if [[ -z "$doc_id" || "$doc_id" == "null" ]]; then
            log "ERROR" "Document type created but id missing. Response: $response_body"
            return 1
        fi
        WS_DOCTYPE_IDS["$txn"]="$doc_id"
        log "SUCCESS" "Document type $txn installed — ID: $doc_id"
        return 0
    fi

    local is_conflict=false
    if [[ "$http_code" -eq 409 ]]; then
        is_conflict=true
    else
        local lower_body
        lower_body=$(echo "$response_body" | tr '[:upper:]' '[:lower:]')
        if echo "$lower_body" | grep -qE 'already (exists|installed)|duplicate'; then
            is_conflict=true
        fi
    fi

    if [[ "$is_conflict" == true ]]; then
        log "WARN" "Document type $txn already exists — retrieving via GET"
        local get_url="https://${B2B_HOST}/b2b/api/v1/documents/${doctype_key}"
        local get_response get_code get_body
        get_response=$(curl -s -w '\n%{http_code}' \
            -X GET \
            --insecure \
            --max-time "$DEFAULT_TIMEOUT" \
            -H "Authorization: Bearer ${MCSP_TOKEN}" \
            "$get_url" 2>&1)

        get_code=$(echo "$get_response" | tail -n1)
        get_body=$(echo "$get_response" | sed '$d')

        if [[ "$get_code" -ge 200 && "$get_code" -lt 300 ]]; then
            local doc_id=${doctype_key}
            WS_DOCTYPE_IDS["$txn"]="$doc_id"
            log "SUCCESS" "Document type $txn already installed — ID: $doc_id"
            return 0
        else
            log "ERROR" "GET fallback for $txn failed (HTTP $get_code): $get_body"
            return 1
        fi
    fi

    log "ERROR" "Failed to install document type $txn (HTTP $http_code): $response_body"
    return 1
}

provision_assets() {
    local domain="$1"

    if is_asset_provisioned; then
        log "SUCCESS" "Assets already provisioned for $domain on $B2B_HOST — skipping"
        return 0
    fi

    log "INFO" "Provisioning assets for domain: $domain on host: $B2B_HOST"
    echo ""

    if [[ -z "$WS_PARTNER1_INTERNAL_ID" ]]; then
        local p1_id
        p1_id=$(create_partner "$DOMAIN_P1_NAME" "$DOMAIN_P1_DUNS" "$DOMAIN_P1_ORG_UNIT") || return 1
        WS_PARTNER1_INTERNAL_ID="$p1_id"
    else
        log "INFO" "Partner 1 already in workspace: $WS_PARTNER1_INTERNAL_ID"
    fi

    if [[ -z "$WS_PARTNER2_INTERNAL_ID" ]]; then
        local p2_id
        p2_id=$(create_partner "$DOMAIN_P2_NAME" "$DOMAIN_P2_DUNS" "$DOMAIN_P2_ORG_UNIT") || return 1
        WS_PARTNER2_INTERNAL_ID="$p2_id"
    else
        log "INFO" "Partner 2 already in workspace: $WS_PARTNER2_INTERNAL_ID"
    fi

    if [[ -z "$WS_CHANNEL_ID" ]]; then
        create_channel "$DOMAIN_CHANNEL_NAME" "p1" || return 1
    else
        log "INFO" "P1 channel already in workspace: $WS_CHANNEL_ID"
    fi
    associate_channel_to_partner "$WS_PARTNER1_INTERNAL_ID" "$WS_CHANNEL_ID" || return 1

    if [[ -z "$WS_PARTNER_USER_ID" ]]; then
        local user_id
        user_id=$(create_partner_user \
            "$DOMAIN_P1_USERNAME" \
            "$DOMAIN_P1_PASSWORD" \
            "$DOMAIN_P1_FULLNAME") || return 1
        WS_PARTNER_USER_ID="$user_id"
    else
        log "INFO" "P1 partner user already in workspace: $WS_PARTNER_USER_ID"
    fi
    associate_user_to_partner \
        "$WS_PARTNER1_INTERNAL_ID" \
        "$WS_PARTNER_USER_ID" \
        "$DOMAIN_P1_USERNAME" || return 1

    if [[ -z "$WS_P2_CHANNEL_ID" ]]; then
        create_channel "$DOMAIN_P2_CHANNEL_NAME" "p2" || return 1
    else
        log "INFO" "P2 channel already in workspace: $WS_P2_CHANNEL_ID"
    fi
    associate_channel_to_partner "$WS_PARTNER2_INTERNAL_ID" "$WS_P2_CHANNEL_ID" || return 1

    if [[ -z "$WS_P2_PARTNER_USER_ID" ]]; then
        local p2_user_id
        p2_user_id=$(create_partner_user \
            "$DOMAIN_P2_USERNAME" \
            "$DOMAIN_P2_PASSWORD" \
            "$DOMAIN_P2_FULLNAME") || return 1
        WS_P2_PARTNER_USER_ID="$p2_user_id"
    else
        log "INFO" "P2 partner user already in workspace: $WS_P2_PARTNER_USER_ID"
    fi
    associate_user_to_partner \
        "$WS_PARTNER2_INTERNAL_ID" \
        "$WS_P2_PARTNER_USER_ID" \
        "$DOMAIN_P2_USERNAME" || return 1

    log "INFO" "Installing document types for domain: $domain"
    for txn in "${DOMAIN_DOC_SEQUENCE[@]}"; do
        if [[ -n "${WS_DOCTYPE_IDS[$txn]:-}" ]]; then
            log "INFO" "Document type $txn already in workspace: ${WS_DOCTYPE_IDS[$txn]}"
            continue
        fi
        install_document_type "$txn" || return 1
        sleep 0.5
    done

    log "SUCCESS" "All assets provisioned for domain: $domain"
    return 0
}


################################################################################
# TRACE VALUE GENERATION
################################################################################

TRACE_RFQ_NUMBER=""
TRACE_QUOTE_NUMBER=""
TRACE_PO_NUMBER=""
TRACE_INVOICE_NUMBER=""
TRACE_SHIPMENT_ID=""
TRACE_PAYMENT_REF=""
TRACE_DELIVERY_DATE=""
TRACE_CLAIM_ID=""
TRACE_TRN_REF=""
TRACE_REMITTANCE_TRACE=""
TRACE_HC_INVOICE_NUMBER=""
TRACE_SC_INVOICE_NUMBER=""

generate_trace_values() {
    local domain="$1"
    local date_stamp
    date_stamp=$(date +%Y%m%d)
    local run_seq
    run_seq=$(printf '%04d' "$WS_RUN_COUNT")

    TRACE_DELIVERY_DATE=$(date -v+14d +%Y%m%d 2>/dev/null || date -d '+14 days' +%Y%m%d 2>/dev/null || echo "${date_stamp}")

    case "$domain" in
        retail)
            TRACE_RFQ_NUMBER="RFQ-${date_stamp}-${run_seq}"
            TRACE_QUOTE_NUMBER="QUOTE-${date_stamp}-${run_seq}"
            TRACE_PO_NUMBER="PO-${date_stamp}-${run_seq}"
            TRACE_INVOICE_NUMBER="INV-${date_stamp}-${run_seq}"
            TRACE_SHIPMENT_ID="SHIP-${date_stamp}-${run_seq}"
            TRACE_PAYMENT_REF="PAY-${date_stamp}-${run_seq}"
            log "INFO" "Retail trace values: RFQ=${TRACE_RFQ_NUMBER}  PO=${TRACE_PO_NUMBER}  INV=${TRACE_INVOICE_NUMBER}"
            ;;
        healthcare)
            TRACE_CLAIM_ID="CLM-${date_stamp}-${run_seq}"
            TRACE_TRN_REF="TRN-${date_stamp}-${run_seq}"
            TRACE_REMITTANCE_TRACE="RMT-${date_stamp}-${run_seq}"
            TRACE_HC_INVOICE_NUMBER="HCINV-${date_stamp}-${run_seq}"
            log "INFO" "Healthcare trace values: CLAIM=${TRACE_CLAIM_ID}  TRN_REF=${TRACE_TRN_REF}  REMIT=${TRACE_REMITTANCE_TRACE}"
            ;;
        supply_chain)
            TRACE_SHIPMENT_ID="SCL-${date_stamp}-${run_seq}"
            TRACE_SC_INVOICE_NUMBER="SCINV-${date_stamp}-${run_seq}"
            log "INFO" "Supply chain trace values: SHIPMENT=${TRACE_SHIPMENT_ID}  INVOICE=${TRACE_SC_INVOICE_NUMBER}"
            ;;
    esac
}

################################################################################
# TEMPLATE RENDERING
################################################################################

render_template() {
    local domain="$1"
    local txn="$2"
    local isa_ctrl
    isa_ctrl=$(printf '%09d' "$3")
    local gs_ctrl="$4"

    local template_dir="templates/${domain}"
    local template_file="${template_dir}/${txn}.edi"

    if [[ ! -f "$template_file" ]]; then
        log "ERROR" "Template not found: $template_file"
        return 1
    fi

    local current_date current_time
    current_date=$(date +%Y%m%d)
    current_time=$(date +%H%M)

    local p1_isa p2_isa
    p1_isa=$(printf '%-15s' "${DOMAIN_P1_DUNS:0:15}")
    p2_isa=$(printf '%-15s' "${DOMAIN_P2_DUNS:0:15}")
    local p1_gs="${DOMAIN_P1_DUNS}"
    local p2_gs="${DOMAIN_P2_DUNS}"

    local tmp_base tmp_file
    tmp_base=$(mktemp "/tmp/b2b_edi_${domain}_${txn}_XXXXXX")
    if [[ -z "$tmp_base" || ! -f "$tmp_base" ]]; then
        log "ERROR" "mktemp failed for $domain/$txn"
        return 1
    fi
    tmp_file="${tmp_base}.edi"
    mv "$tmp_base" "$tmp_file"

    sed \
        -e "s|{{ISA_CTRL}}|${isa_ctrl}|g" \
        -e "s|{{GS_CTRL}}|${gs_ctrl}|g" \
        -e "s|{{ISA_DATE}}|${current_date:2}|g" \
        -e "s|{{DATE}}|${current_date}|g" \
        -e "s|{{TIME}}|${current_time}|g" \
        -e "s|{{P1_ISA}}|${p1_isa}|g" \
        -e "s|{{P2_ISA}}|${p2_isa}|g" \
        -e "s|{{P1_GS}}|${p1_gs}|g" \
        -e "s|{{P2_GS}}|${p2_gs}|g" \
        -e "s|{{DELIVERY_DATE}}|${TRACE_DELIVERY_DATE}|g" \
        -e "s|{{RFQ_NUMBER}}|${TRACE_RFQ_NUMBER}|g" \
        -e "s|{{QUOTE_NUMBER}}|${TRACE_QUOTE_NUMBER}|g" \
        -e "s|{{PO_NUMBER}}|${TRACE_PO_NUMBER}|g" \
        -e "s|{{INVOICE_NUMBER}}|${TRACE_INVOICE_NUMBER}|g" \
        -e "s|{{SHIPMENT_ID}}|${TRACE_SHIPMENT_ID}|g" \
        -e "s|{{PAYMENT_REF}}|${TRACE_PAYMENT_REF}|g" \
        -e "s|{{CLAIM_ID}}|${TRACE_CLAIM_ID}|g" \
        -e "s|{{TRN_REF}}|${TRACE_TRN_REF}|g" \
        -e "s|{{REMITTANCE_TRACE}}|${TRACE_REMITTANCE_TRACE}|g" \
        -e "s|{{HC_INVOICE_NUMBER}}|${TRACE_HC_INVOICE_NUMBER}|g" \
        -e "s|{{SC_INVOICE_NUMBER}}|${TRACE_SC_INVOICE_NUMBER}|g" \
        "$template_file" > "$tmp_file"

    echo "$tmp_file"
    return 0
}

################################################################################
# DOCUMENT SUBMISSION
################################################################################

INTER_DOCUMENT_DELAY=2

post_edi_document() {
    local edi_file="$1"
    local txn="$2"

    local isa06_raw isa06
    isa06_raw=$(grep -m1 '^ISA' "$edi_file" | cut -d'*' -f7)
    isa06=$(echo "$isa06_raw" | sed 's/ *$//')

    local channel_uri auth sender_label
    local p1_duns_trimmed p2_duns_trimmed
    p1_duns_trimmed=$(echo "$DOMAIN_P1_DUNS" | sed 's/ *$//')
    p2_duns_trimmed=$(echo "$DOMAIN_P2_DUNS" | sed 's/ *$//')

    if [[ "$isa06" == "$p1_duns_trimmed" ]]; then
        channel_uri="$WS_CHANNEL_ENDPOINT_URI"
        auth="${DOMAIN_P1_USERNAME}:${DOMAIN_P1_PASSWORD}"
        sender_label="P1 (${DOMAIN_P1_NAME})"
    elif [[ "$isa06" == "$p2_duns_trimmed" ]]; then
        channel_uri="$WS_P2_CHANNEL_ENDPOINT_URI"
        auth="${DOMAIN_P2_USERNAME}:${DOMAIN_P2_PASSWORD}"
        sender_label="P2 (${DOMAIN_P2_NAME})"
    else
        log "ERROR" "ISA06 DUNS '$isa06' does not match P1 ('$p1_duns_trimmed') or P2 ('$p2_duns_trimmed') — cannot route $txn"
        return 1
    fi

    log "INFO" "Routing $txn → sender: $sender_label (ISA06=$isa06)"

    if [[ -z "$channel_uri" ]]; then
        log "ERROR" "Channel endpoint URI is empty for $sender_label — run --feature setup --domain $domain first."
        return 1
    fi

    local url
    if [[ "$channel_uri" == https://* || "$channel_uri" == http://* ]]; then
        url="$channel_uri"
    else
        url="https://${channel_uri}"
    fi

    log "INFO" "POSTing $txn to $url"

    local response
    response=$(curl -s -w '\n%{http_code}' \
        -X POST \
        --insecure \
        --max-time "$POST_PAYLOAD_TIMEOUT" \
        -u "$auth" \
        -H "Content-Type: application/EDI" \
        --data-binary "@${edi_file}" \
        "$url" 2>&1)

    local http_code response_body
    http_code=$(echo "$response" | tail -n1)
    response_body=$(echo "$response" | sed '$d')

    if [[ "$http_code" -ge 200 && "$http_code" -lt 300 ]]; then
        log "SUCCESS" "Document $txn submitted (HTTP $http_code)"
        return 0
    else
        log "ERROR" "Document $txn failed (HTTP $http_code): $response_body"
        return 1
    fi
}

submit_document_sequence() {
    local domain="$1"
    local docs_filter="${2:-}"
    local dry_run="${3:-false}"

    log "INFO" "Starting document submission for domain: $domain  (dry_run=${dry_run})"
    echo ""

    local current_isa=$WS_ISA_CTRL_NUM
    local current_gs=$WS_GS_CTRL_NUM
    local submitted=0
    local failed=0
    local run_log="${WORKSPACE_DIR}/${domain}_runs.log"

    local -a txn_list=()
    if [[ -n "$docs_filter" ]]; then
        IFS=',' read -ra txn_list <<< "$docs_filter"
    else
        txn_list=("${DOMAIN_DOC_SEQUENCE[@]}")
    fi

    for txn in "${txn_list[@]}"; do
        echo "──────────────────────────────────────────────────────────────"
        log "INFO" "Processing document: $txn  (ISA ctrl: $(printf '%09d' $current_isa))"

        local rendered_file
        rendered_file=$(render_template "$domain" "$txn" "$current_isa" "$current_gs") || {
            log "ERROR" "Skipping $txn — template render failed"
            (( failed++ )) || true
            continue
        }

        if [[ "$dry_run" == "true" ]]; then
            log "INFO" "[DRY RUN] Rendered $txn — content preview:"
            echo "--- BEGIN $txn ---"
            cat "$rendered_file"
            echo ""
            echo "--- END $txn ---"
            rm -f "$rendered_file"
        else
            if post_edi_document "$rendered_file" "$txn"; then
                (( submitted++ )) || true
                echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') domain=${domain} txn=${txn} isa=${current_isa} status=SUCCESS" >> "$run_log"
            else
                (( failed++ )) || true
                echo "$(date -u '+%Y-%m-%dT%H:%M:%SZ') domain=${domain} txn=${txn} isa=${current_isa} status=FAILED" >> "$run_log"
            fi
            rm -f "$rendered_file"
            [[ $txn != "${txn_list[-1]}" ]] && sleep "$INTER_DOCUMENT_DELAY"
        fi

        (( current_isa++ )) || true
        (( current_gs++ ))  || true
    done

    WS_ISA_CTRL_NUM=$current_isa
    WS_GS_CTRL_NUM=$current_gs

    echo ""
    echo "══════════════════════════════════════════════════════════════"
    if [[ "$dry_run" == "true" ]]; then
        log "INFO" "DRY RUN complete — ${#txn_list[@]} template(s) rendered, nothing submitted"
    else
        log "INFO" "Submission complete — submitted: $submitted  failed: $failed"
        [[ $failed -gt 0 ]] && log "WARN" "Run log: $run_log"
    fi
    echo "══════════════════════════════════════════════════════════════"

    return $failed
}

################################################################################
# CLI INTERFACE
################################################################################

usage() {
    cat << EOF

${BLUE}IBM webMethods B2B Toolkit${NC}

${YELLOW}CONFIGURATION (edit toolkit.env — never edit this script):${NC}
  B2B_HOST                  B2B SaaS hostname
  MCSP_INSTANCE_ID          MCSP account UUID
  MCSP_API_KEY              Base64-encoded MCSP API key
  RETAIL_P1_NAME / _DUNS /  Per-domain partner names, DUNS IDs, credentials
  HEALTHCARE_* / SUPPLY_*   (see demo.env.example for the full variable list)

${YELLOW}USAGE:${NC}
  $0 --feature FEATURE --domain DOMAIN [OPTIONS]
  $0 --feature FEATURE --all-domains   [OPTIONS]
  $0 --list-domains
  $0 --status DOMAIN
  $0 --reset  DOMAIN

${YELLOW}FEATURES (--feature):${NC}
  setup     Provision B2B assets only (partners, channels, users, document
            types). Idempotent — safe to run multiple times. No documents
            are submitted. Useful as a standalone provisioning step before
            any other workflow.

  trace     Full EDI transaction-linking demo. Provisions assets (if not
            already done) then submits a complete multi-document EDI trace
            sequence for the chosen domain.

${YELLOW}REQUIRED:${NC}
  -f, --feature FEATURE     Feature to run: setup | trace
  -d, --domain  DOMAIN      Target domain:  retail | healthcare | supply_chain
      --all-domains         Run all three domains in sequence (trace feature)

${YELLOW}OPTIONS (trace feature):${NC}
  --submit-only             Submit documents only (assets must already exist)
  --dry-run                 Render all templates and print them; do not submit
  --docs TXN[,TXN,...]      Submit only the specified transaction codes
                            e.g. --docs 840,843,850
  --delay SECONDS           Pause between document submissions (default: ${INTER_DOCUMENT_DELAY}s)

${YELLOW}UTILITY COMMANDS (no --feature required):${NC}
  --list-domains            List available domains and their workspace status
  --status DOMAIN           Print workspace state for the given domain
  --reset DOMAIN            Clear workspace state for the given domain
  -h, --help                Show this help message

${YELLOW}EXAMPLES:${NC}
  # Provision B2B assets for retail (partners, channels, doc types)
  $0 --feature setup --domain retail

  # Full retail EDI trace — provision + submit all 9 documents
  $0 --feature trace --domain retail

  # Healthcare trace — submit only (assets already provisioned)
  $0 --feature trace --domain healthcare --submit-only

  # Supply chain — dry run (print rendered EDI, no API calls)
  $0 --feature trace --domain supply_chain --dry-run

  # Run all three domains back-to-back (trace)
  $0 --feature trace --all-domains

  # Submit a subset of retail documents
  $0 --feature trace --domain retail --submit-only --docs 840,843,850

  # Show workspace state for healthcare
  $0 --status healthcare

  # Reset retail workspace (re-provisions on next run)
  $0 --reset retail

${YELLOW}WORKSPACE:${NC}
  State files are stored under  .b2b-toolkit/<hostname>/<domain>.json
  The hostname key is derived from B2B_HOST, so the same toolkit file can
  target multiple B2B instances without conflict.

EOF
}

list_domains() {
    echo ""
    echo "Available domains on host: ${B2B_HOST}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    local sanitized_host
    sanitized_host=$(echo "$B2B_HOST" | tr '.' '-' | tr '[:upper:]' '[:lower:]')
    local ws_root="${WORKSPACE_ROOT}/${sanitized_host}"

    for d in retail healthcare supply_chain; do
        local ws_file="${ws_root}/${d}.json"
        if [[ -f "$ws_file" ]]; then
            local run_count last_run
            run_count=$(jq -r '.counters.run_count // 0' "$ws_file")
            last_run=$(jq -r '.last_run // "(never)"' "$ws_file")
            printf "  %-20s %s  (runs: %s, last: %s)\n" "$d" "[provisioned]" "$run_count" "$last_run"
        else
            printf "  %-20s %s\n" "$d" "[not provisioned]"
        fi
    done

    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
}

reset_domain() {
    local domain="$1"
    load_domain_config "$domain" || exit 1
    resolve_workspace_path "$domain"

    if [[ ! -f "$WORKSPACE_FILE" ]]; then
        log "WARN" "No workspace file found for $domain on $B2B_HOST — nothing to reset"
        return 0
    fi

    echo ""
    echo -e "${YELLOW}WARNING:${NC} This will delete the workspace state for domain '${domain}' on host '${B2B_HOST}'."
    echo "  File: $WORKSPACE_FILE"
    echo ""
    read -r -p "Are you sure? Type 'yes' to confirm: " confirm
    if [[ "$confirm" != "yes" ]]; then
        log "INFO" "Reset cancelled"
        return 0
    fi

    rm -f "$WORKSPACE_FILE"
    log "SUCCESS" "Workspace reset for domain: $domain on host: $B2B_HOST"
    echo "  On the next run, assets will be re-provisioned from scratch."
    echo ""
}

print_run_summary() {
    local domain="$1"
    local feature="$2"
    local mode="$3"
    local docs_count="$4"
    local ts
    ts=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

    echo ""
    echo "══════════════════════════════════════════════════════════════"
    echo "  Run Summary"
    echo "══════════════════════════════════════════════════════════════"
    printf "  %-22s %s\n" "Timestamp:"     "$ts"
    printf "  %-22s %s\n" "Host:"          "$B2B_HOST"
    printf "  %-22s %s\n" "Feature:"       "$feature"
    printf "  %-22s %s\n" "Domain:"        "$domain"
    printf "  %-22s %s\n" "Mode:"          "$mode"
    printf "  %-22s %s\n" "Run count:"     "$WS_RUN_COUNT"
    printf "  %-22s %s\n" "Docs targeted:" "$docs_count"
    printf "  %-22s %s\n" "Workspace:"     "$WORKSPACE_FILE"

    local trace_summary=""
    [[ -n "$TRACE_RFQ_NUMBER"        ]] && trace_summary+="    RFQNumber:         $TRACE_RFQ_NUMBER\n"
    [[ -n "$TRACE_QUOTE_NUMBER"      ]] && trace_summary+="    QuoteNumber:       $TRACE_QUOTE_NUMBER\n"
    [[ -n "$TRACE_PO_NUMBER"         ]] && trace_summary+="    PONumber:          $TRACE_PO_NUMBER\n"
    [[ -n "$TRACE_INVOICE_NUMBER"    ]] && trace_summary+="    InvoiceNumber:     $TRACE_INVOICE_NUMBER\n"
    [[ -n "$TRACE_SHIPMENT_ID"       ]] && trace_summary+="    ShipmentID:        $TRACE_SHIPMENT_ID\n"
    [[ -n "$TRACE_PAYMENT_REF"       ]] && trace_summary+="    PaymentRef:        $TRACE_PAYMENT_REF\n"
    [[ -n "$TRACE_CLAIM_ID"          ]] && trace_summary+="    ClaimID:           $TRACE_CLAIM_ID\n"
    [[ -n "$TRACE_TRN_REF"           ]] && trace_summary+="    TRN_Ref:           $TRACE_TRN_REF\n"
    [[ -n "$TRACE_REMITTANCE_TRACE"  ]] && trace_summary+="    RemittanceTrace:   $TRACE_REMITTANCE_TRACE\n"
    [[ -n "$TRACE_SC_INVOICE_NUMBER" ]] && trace_summary+="    SC_InvoiceNumber:  $TRACE_SC_INVOICE_NUMBER\n"

    if [[ -n "$trace_summary" ]]; then
        echo "  Linking attributes used:"
        echo -e "$trace_summary"
    fi
    echo "══════════════════════════════════════════════════════════════"
    echo ""
}

################################################################################
# FEATURE: setup
# Provision B2B assets for a domain — idempotent, no document submission.
################################################################################

run_feature_setup() {
    local domain="$1"

    log "INFO" "Feature: setup | Domain: $domain"

    load_domain_config "$domain" || exit 1
    resolve_workspace_path "$domain"
    load_workspace "$domain"
    acquire_mcsp_token || exit 1

    provision_assets "$domain" || exit 1
    save_workspace "$domain"

    print_run_summary "$domain" "setup" "provision-only" "0"
    echo ""
    log "SUCCESS" "Setup complete for domain: $domain on $B2B_HOST"
}

################################################################################
# FEATURE: trace
# Provision assets (if needed) then submit a full EDI trace sequence.
################################################################################

run_feature_trace() {
    local domain="$1"
    local mode="$2"
    local docs_filter="$3"
    local all_domains="$4"

    # ── --all-domains: re-invoke per domain for clean state isolation ─────────
    if [[ "$all_domains" == true ]]; then
        log "INFO" "Running all domains: retail → healthcare → supply_chain"
        local _overall_exit=0
        for _d in retail healthcare supply_chain; do
            echo ""
            echo "████████████████████████████████████████████████████████████"
            echo "  Feature: trace | Domain: $_d"
            echo "████████████████████████████████████████████████████████████"
            local _args=("--feature" "trace" "--domain" "$_d")
            [[ "$mode" != "full" ]]     && _args+=("--${mode}")
            [[ -n "$docs_filter" ]]     && _args+=("--docs" "$docs_filter")
            [[ "$INTER_DOCUMENT_DELAY" != "2" ]] && _args+=("--delay" "$INTER_DOCUMENT_DELAY")
            bash "${BASH_SOURCE[0]}" "${_args[@]}" || _overall_exit=$?
        done
        exit $_overall_exit
    fi

    log "INFO" "Feature: trace | Domain: $domain | Mode: $mode"

    load_domain_config "$domain" || exit 1
    resolve_workspace_path "$domain"
    load_workspace "$domain"

    local -a target_docs=()
    if [[ -n "$docs_filter" ]]; then
        IFS=',' read -ra target_docs <<< "$docs_filter"
    else
        target_docs=("${DOMAIN_DOC_SEQUENCE[@]}")
    fi

    # MCSP token is only needed for provisioning (B2B management API calls).
    # Document submission uses partner-user Basic Auth directly — no token needed.
    if [[ "$mode" == "full" ]]; then
        acquire_mcsp_token || exit 1
        provision_assets "$domain" || exit 1
        save_workspace "$domain"
    fi

    if [[ "$mode" == "submit-only" ]]; then
        if ! is_asset_provisioned; then
            log "ERROR" "Assets not provisioned for $domain. Run --feature setup --domain $domain first."
            exit 1
        fi
    fi

    (( WS_RUN_COUNT++ )) || true
    generate_trace_values "$domain"

    local dry_run_flag="false"
    [[ "$mode" == "dry-run" ]] && dry_run_flag="true"

    submit_document_sequence "$domain" "$docs_filter" "$dry_run_flag"
    local submit_exit=$?

    if [[ "$dry_run_flag" != "true" ]]; then
        save_workspace "$domain"
    fi

    print_run_summary "$domain" "trace" "$mode" "${#target_docs[@]}"
    exit $submit_exit
}

################################################################################
# MAIN
################################################################################

main() {
    display_banner

    if ! check_dependencies; then
        exit 1
    fi

    local feature=""
    local domain=""
    local mode="full"
    local docs_filter=""
    local cmd=""
    local all_domains=false

    while [[ $# -gt 0 ]]; do
        case "$1" in
            -f|--feature)
                feature="$2"; shift 2 ;;
            -d|--domain)
                domain="$2"; shift 2 ;;
            --all-domains)
                all_domains=true; shift ;;
            --submit-only)
                mode="submit-only"; shift ;;
            --dry-run)
                mode="dry-run"; shift ;;
            --docs)
                docs_filter="$2"; shift 2 ;;
            --delay)
                INTER_DOCUMENT_DELAY="$2"; shift 2 ;;
            --list-domains)
                cmd="list-domains"; shift ;;
            --status)
                cmd="status"; domain="$2"; shift 2 ;;
            --reset)
                cmd="reset"; domain="$2"; shift 2 ;;
            -h|--help)
                usage; exit 0 ;;
            *)
                log "ERROR" "Unknown option: $1"
                usage; exit 1 ;;
        esac
    done

    # ── Utility commands (no --feature required) ──────────────────────────────
    if [[ "$cmd" == "list-domains" ]]; then
        list_domains
        exit 0
    fi

    if [[ "$cmd" == "status" ]]; then
        [[ -z "$domain" ]] && { log "ERROR" "--status requires a domain name"; exit 1; }
        load_domain_config "$domain" || exit 1
        resolve_workspace_path "$domain"
        load_workspace "$domain"
        print_workspace_status "$domain"
        exit 0
    fi

    if [[ "$cmd" == "reset" ]]; then
        [[ -z "$domain" ]] && { log "ERROR" "--reset requires a domain name"; exit 1; }
        reset_domain "$domain"
        exit 0
    fi

    # ── Feature is required for all other invocations ─────────────────────────
    if [[ -z "$feature" ]]; then
        log "ERROR" "--feature is required. Valid features: setup | trace"
        usage
        exit 1
    fi

    case "$feature" in
        setup)
            [[ -z "$domain" ]] && { log "ERROR" "--feature setup requires --domain"; usage; exit 1; }
            run_feature_setup "$domain"
            ;;
        trace)
            if [[ "$all_domains" == false && -z "$domain" ]]; then
                log "ERROR" "--feature trace requires --domain (or --all-domains)"
                usage
                exit 1
            fi
            run_feature_trace "$domain" "$mode" "$docs_filter" "$all_domains"
            ;;
        *)
            log "ERROR" "Unknown feature: '$feature'. Valid features: setup | trace"
            usage
            exit 1
            ;;
    esac
}

################################################################################
# SCRIPT ENTRY POINT
################################################################################

main "$@"
