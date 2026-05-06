#!/bin/bash
set -euo pipefail

# Lumen transparent capture -- pf rule setup for macOS
#
# Redirects outbound HTTPS traffic to monitored LLM API hosts through
# Lumen's transparent proxy on localhost:9443.
#
# Loop avoidance: traffic from UID _lumen (or the specified UID) is
# excluded from interception so the proxy's upstream connections pass
# through unmodified.
#
# Requires root. Uses a named pf anchor (com.datagrout.lumen) to avoid
# clobbering existing firewall rules.

ANCHOR="com.datagrout.lumen"
PROXY_PORT="${LUMEN_TRANSPARENT_PORT:-9443}"
EXCLUDE_USER="${LUMEN_PROXY_USER:-root}"

# LLM API hosts to intercept
HOSTS=(
    "api.openai.com"
    "api.anthropic.com"
    "generativelanguage.googleapis.com"
    "api2.cursor.sh"
    "api3.cursor.sh"
)

usage() {
    cat <<EOF
Usage: sudo $0 [options]

Options:
  --port PORT     Transparent proxy port (default: 9443)
  --user USER     UID/username to exclude from interception (default: root)
  --hosts FILE    File with additional hosts to intercept (one per line)
  --interface IF  Network interface (default: auto-detect active)
  --local         Also intercept local traffic (requires --user for loop avoidance)
  --teardown      Remove all Lumen pf rules and exit

Managed anchor: ${ANCHOR}
EOF
    exit 0
}

# Parse arguments
LOCAL_TRAFFIC=false
EXTRA_HOSTS_FILE=""
INTERFACE=""
TEARDOWN=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --port) PROXY_PORT="$2"; shift 2 ;;
        --user) EXCLUDE_USER="$2"; shift 2 ;;
        --hosts) EXTRA_HOSTS_FILE="$2"; shift 2 ;;
        --interface) INTERFACE="$2"; shift 2 ;;
        --local) LOCAL_TRAFFIC=true; shift ;;
        --teardown) TEARDOWN=true; shift ;;
        -h|--help) usage ;;
        *) echo "Unknown option: $1"; usage ;;
    esac
done

if $TEARDOWN; then
    echo "==> Flushing Lumen pf anchor '${ANCHOR}'"
    pfctl -a "${ANCHOR}" -F all 2>/dev/null || true
    echo "Done. Anchor '${ANCHOR}' flushed."
    echo "Note: pf left enabled. Run 'sudo pfctl -d' to disable pf entirely."
    exit 0
fi

# Auto-detect active network interface
if [ -z "$INTERFACE" ]; then
    INTERFACE=$(route -n get default 2>/dev/null | awk '/interface:/{print $2}')
    if [ -z "$INTERFACE" ]; then
        INTERFACE="en0"
    fi
fi

echo "==> Lumen transparent capture setup"
echo "    Interface:    ${INTERFACE}"
echo "    Proxy port:   ${PROXY_PORT}"
echo "    Exclude user: ${EXCLUDE_USER}"
echo "    Local traffic: ${LOCAL_TRAFFIC}"
echo ""

# Load additional hosts from file
if [ -n "$EXTRA_HOSTS_FILE" ] && [ -f "$EXTRA_HOSTS_FILE" ]; then
    while IFS= read -r line; do
        line=$(echo "$line" | xargs) # trim whitespace
        [[ -z "$line" || "$line" == \#* ]] && continue
        HOSTS+=("$line")
    done < "$EXTRA_HOSTS_FILE"
fi

# Resolve hosts to IPs
echo "==> Resolving monitored hosts"
DEST_IPS=()
for host in "${HOSTS[@]}"; do
    ips=$(dig +short "$host" A 2>/dev/null | grep -E '^[0-9]+\.' || true)
    if [ -z "$ips" ]; then
        echo "    WARN: Could not resolve ${host}, skipping"
        continue
    fi
    while IFS= read -r ip; do
        echo "    ${host} -> ${ip}"
        DEST_IPS+=("$ip")
    done <<< "$ips"
done

if [ ${#DEST_IPS[@]} -eq 0 ]; then
    echo "ERROR: No IPs resolved. Cannot create redirect rules."
    exit 1
fi

# Build pf table of destination IPs
IP_TABLE=$(printf ", %s" "${DEST_IPS[@]}")
IP_TABLE="{ ${IP_TABLE:2} }"

# Build anchor rules
RULES=""

# Redirect forwarded traffic (from LAN clients through this machine)
RULES+="rdr on ${INTERFACE} proto tcp from any to ${IP_TABLE} port 443 -> 127.0.0.1 port ${PROXY_PORT}"
RULES+=$'\n'

if $LOCAL_TRAFFIC; then
    # Redirect locally-originated traffic
    RULES+="rdr on lo0 proto tcp from any to ${IP_TABLE} port 443 -> 127.0.0.1 port ${PROXY_PORT}"
    RULES+=$'\n'
    # Route local outbound to lo0 (so rdr on lo0 catches it), EXCEPT from the proxy user
    RULES+="pass out on ${INTERFACE} route-to (lo0 127.0.0.1) proto tcp from any to ${IP_TABLE} port 443 user != ${EXCLUDE_USER}"
    RULES+=$'\n'
fi

echo ""
echo "==> Loading pf anchor '${ANCHOR}'"
echo "$RULES"

# Load anchor rules
echo "$RULES" | pfctl -a "${ANCHOR}" -f /dev/stdin 2>&1

# Ensure the main ruleset references our anchor (required for pf to evaluate it).
# pf requires strict ordering: translation (rdr-anchor) before filtering (anchor).
PF_CONF=$(cat /etc/pf.conf 2>/dev/null || echo "")

if ! echo "$PF_CONF" | grep -q "rdr-anchor \"${ANCHOR}\""; then
    echo "==> Adding anchor reference to main pf ruleset"
    # Use awk to insert our rules right after the com.apple counterparts
    PF_CONF=$(echo "$PF_CONF" | awk -v anchor="${ANCHOR}" '
        { print }
        /^rdr-anchor "com\.apple/ && !rdr_done { print "rdr-anchor \"" anchor "\""; rdr_done=1 }
        /^anchor "com\.apple/ && !anc_done { print "anchor \"" anchor "\""; anc_done=1 }
    ')
    echo "$PF_CONF" | pfctl -f /dev/stdin 2>&1 || true
fi

# Enable pf (harmless if already enabled)
pfctl -e 2>/dev/null || true

echo ""
echo "==> Verifying"
pfctl -a "${ANCHOR}" -s rules 2>/dev/null
echo ""
echo "Done. Lumen transparent capture active."
echo "  Intercepting: ${HOSTS[*]}"
echo "  Proxy:        127.0.0.1:${PROXY_PORT}"
echo ""
echo "To remove: sudo $0 --teardown"
