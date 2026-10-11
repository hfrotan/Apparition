#!/bin/bash
# probe-lan.sh <endpoint> [json-args]: call Apparition on the headset by LAN address,
# for when Tailscale is off (LocalDevVPN holds the VPN slot) and the apps MCP cannot
# reach http://avp. Uses Apple's curl: macOS Local Network privacy blocks Homebrew
# binaries and Xcode's python. APPARITION_HOST overrides the address.
rec="$HOME/.local/state/debugtrace/sessions/com.illixion.apparition@AVP.json"
hdr=$(mktemp); trap 'rm -f "$hdr"' EXIT
/usr/bin/plutil -extract token raw -o - "$rec" | sed 's/^/Authorization: Bearer /' > "$hdr"
case "$1" in probeState|_logs) m=GET;; *) m=POST;; esac
/usr/bin/curl -s -m 90 -X "$m" -H @"$hdr" -H 'Content-Type: application/json' -H 'X-DebugTrace-Client: claude-probe' \
  ${2:+--data "$2"} "http://${APPARITION_HOST:-172.20.48.129}:8644/$1"
