#!/bin/sh
# One-step installer for AtomDNS + the customized sing-box (DefaWong/atomdns).
#
#   curl -fsSL https://raw.githubusercontent.com/DefaWong/atomdns/main/scripts/install.sh | sh
#   wget -qO- https://raw.githubusercontent.com/DefaWong/atomdns/main/scripts/install.sh | sh
#
# Supports Alpine (OpenRC) and Debian/Ubuntu (systemd) on x86_64. Installs both programs,
# a fresh AtomDNS configuration (sing-box FakeIP upstream, sing-box API pre-wired) and a
# sing-box configuration with a TProxy inbound for the FakeIP ranges. Re-running upgrades
# the programs and the WebUI but keeps existing configuration.
#
# Environment overrides:
#   ATOMDNS_RELEASE   release tag to install (default: latest)
#   ATOMDNS_REPO      GitHub repository (default: DefaWong/atomdns)
#   ATOMDNS_ASSETS    local directory holding the release assets instead of downloading
#   ATOMDNS_ROOT      AtomDNS configuration root (default: /cus/atomdns)
#   TPROXY_PORT       sing-box TProxy port (default: 7895)
set -eu

REPO="${ATOMDNS_REPO:-DefaWong/atomdns}"
RELEASE="${ATOMDNS_RELEASE:-latest}"
ROOT="${ATOMDNS_ROOT:-/cus/atomdns}"
TPROXY_PORT="${TPROXY_PORT:-7895}"
SINGBOX_DIR=/etc/sing-box
SINGBOX_STATE=/var/lib/sing-box
LIB_DIR=/usr/local/lib/atomdns
BIN_DIR=/usr/local/bin
FAKEIP4=198.18.0.0/15
FAKEIP6=fc00::/18
API_PORT=9090
FAKEIP_DNS_PORT=6666
MIXED_PORT=9666
TPROXY_MARK=0x1d
TPROXY_TABLE=233

log() { printf '\033[32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[33mwarning:\033[0m %s\n' "$*" >&2; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# --- platform -------------------------------------------------------------------------

[ "$(id -u)" = 0 ] || die "run as root"
[ "$(uname -m)" = x86_64 ] || die "only x86_64 (amd64) is supported, this machine is $(uname -m)"

if [ -f /etc/alpine-release ]; then
	OS=alpine
	command -v openrc-run >/dev/null 2>&1 || die "Alpine without OpenRC is not supported"
elif [ -f /etc/debian_version ]; then
	OS=debian
	command -v systemctl >/dev/null 2>&1 || die "Debian without systemd is not supported"
else
	die "unsupported system: only Alpine and Debian/Ubuntu are supported"
fi

install_packages() {
	log "installing dependencies"
	if [ "$OS" = alpine ]; then
		apk add --no-cache ca-certificates curl tar unzip nftables iproute2 >/dev/null
	else
		export DEBIAN_FRONTEND=noninteractive
		apt-get update -qq >/dev/null
		apt-get install -y -qq ca-certificates curl tar unzip nftables iproute2 >/dev/null
	fi
}

service_running() {
	if [ "$OS" = alpine ]; then
		rc-service "$1" status >/dev/null 2>&1
	else
		systemctl is-active --quiet "$1"
	fi
}

# Port 53 must be free unless it is our own AtomDNS (re-run / upgrade).
check_dns_port() {
	if service_running atomdns; then
		return
	fi
	if ss -lnu 2>/dev/null | awk '{print $4}' | grep -Eq '(^|[:.])53$'; then
		ss -lnup 2>/dev/null | grep -E '[:.]53[[:space:]]' >&2 || true
		die "UDP port 53 is already in use (systemd-resolved stub, dnsmasq ...); free it and re-run"
	fi
}

# --- assets ---------------------------------------------------------------------------

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

ASSETS="atomdns-linux-amd64-musl.tar.gz sing-box-linux-amd64.tar.gz atomdns-webui.tar.gz config_rust.zip SHA256SUMS"

fetch_assets() {
	if [ -n "${ATOMDNS_ASSETS:-}" ]; then
		log "using local assets from ${ATOMDNS_ASSETS}"
		for asset in $ASSETS; do
			cp "${ATOMDNS_ASSETS}/${asset}" "$WORK/" || die "missing ${ATOMDNS_ASSETS}/${asset}"
		done
	else
		if [ "$RELEASE" = latest ]; then
			base="https://github.com/${REPO}/releases/latest/download"
		else
			base="https://github.com/${REPO}/releases/download/${RELEASE}"
		fi
		log "downloading ${REPO} ${RELEASE}"
		for asset in $ASSETS; do
			curl -fsSL --retry 3 -o "$WORK/${asset}" "${base}/${asset}" || die "download failed: ${base}/${asset}"
		done
	fi
	(cd "$WORK" && sha256sum -c SHA256SUMS >/dev/null) || die "sha256 verification failed"
}

# --- programs -------------------------------------------------------------------------

install_programs() {
	log "installing programs to ${BIN_DIR}"
	mkdir -p "$WORK/atomdns" "$WORK/sing-box"
	tar -xzf "$WORK/atomdns-linux-amd64-musl.tar.gz" -C "$WORK/atomdns"
	tar -xzf "$WORK/sing-box-linux-amd64.tar.gz" -C "$WORK/sing-box"
	install -m 0755 "$WORK/atomdns/atomdns" "${BIN_DIR}/atomdns.new"
	install -m 0755 "$WORK/sing-box/sing-box" "${BIN_DIR}/sing-box.new"
	"${BIN_DIR}/atomdns.new" version >/dev/null || die "the downloaded atomdns does not run here"
	"${BIN_DIR}/sing-box.new" version >/dev/null || die "the downloaded sing-box does not run here"
}

# Swapped in only while the services are stopped.
activate_programs() {
	mv -f "${BIN_DIR}/atomdns.new" "${BIN_DIR}/atomdns"
	mv -f "${BIN_DIR}/sing-box.new" "${BIN_DIR}/sing-box"
}

random_secret() {
	head -c 16 /dev/urandom | od -An -tx1 | tr -d ' \n'
}

# --- AtomDNS configuration ------------------------------------------------------------

install_atomdns_config() {
	if [ -f "${ROOT}/config_custom.yaml" ]; then
		log "keeping existing AtomDNS configuration in ${ROOT}"
	else
		log "creating AtomDNS configuration in ${ROOT}"
		mkdir -p "$ROOT"
		unzip -q -o "$WORK/config_rust.zip" -d "$ROOT"
	fi
	# The WebUI always follows the installed program.
	rm -rf "${ROOT}/ui/main.new"
	mkdir -p "${ROOT}/ui/main.new"
	tar -xzf "$WORK/atomdns-webui.tar.gz" -C "${ROOT}/ui/main.new"
	rm -rf "${ROOT}/ui/main"
	mv "${ROOT}/ui/main.new" "${ROOT}/ui/main"

	if [ ! -f "${ROOT}/webinfo/settings.json" ]; then
		mkdir -p "${ROOT}/webinfo"
		umask 077
		cat >"${ROOT}/webinfo/settings.json" <<EOF
{
  "version": 1,
  "proxies": {
    "main_proxy": {
      "addr": "127.0.0.1:${MIXED_PORT}"
    }
  },
  "singbox_api": {
    "addr": "http://127.0.0.1:${API_PORT}",
    "secret": "${SINGBOX_SECRET}"
  }
}
EOF
		umask 022
	fi
	# It holds the sing-box API secret.
	chmod 0600 "${ROOT}/webinfo/settings.json"
}

# --- sing-box configuration -----------------------------------------------------------

install_singbox_config() {
	mkdir -p "$SINGBOX_DIR" "$SINGBOX_STATE"
	chmod 0700 "$SINGBOX_DIR" "$SINGBOX_STATE"
	if [ -f "${SINGBOX_DIR}/config.json" ]; then
		log "keeping existing sing-box configuration in ${SINGBOX_DIR}"
		return
	fi
	log "creating sing-box configuration in ${SINGBOX_DIR}"
	umask 077
	cat >"${SINGBOX_DIR}/config.json" <<EOF
{
  "log": {"level": "info", "timestamp": true},
  "dns": {
    "servers": [
      {"type": "udp", "tag": "local", "server": "223.5.5.5"},
      {"type": "fakeip", "tag": "fakeip", "inet4_range": "${FAKEIP4}", "inet6_range": "${FAKEIP6}"}
    ],
    "rules": [
      {"inbound": "atomdns-fakeip-in", "query_type": ["A", "AAAA"], "action": "route", "server": "fakeip"}
    ],
    "final": "local"
  },
  "inbounds": [
    {"type": "direct", "tag": "atomdns-fakeip-in", "listen": "127.0.0.1", "listen_port": ${FAKEIP_DNS_PORT}, "override_port": 53},
    {"type": "mixed", "tag": "atomdns-proxy-in", "listen": "127.0.0.1", "listen_port": ${MIXED_PORT}},
    {"type": "tproxy", "tag": "tproxy-in", "listen": "::", "listen_port": ${TPROXY_PORT}}
  ],
  "outbounds": [
    {"type": "selector", "tag": "proxy", "outbounds": ["direct"], "default": "direct", "interrupt_exist_connections": false},
    {"type": "direct", "tag": "direct"}
  ],
  "route": {
    "rules": [
      {"inbound": "atomdns-fakeip-in", "action": "hijack-dns"}
    ],
    "final": "proxy",
    "default_domain_resolver": "local"
  },
  "experimental": {
    "cache_file": {"enabled": true, "path": "${SINGBOX_STATE}/cache.db", "store_fakeip": true},
    "clash_api": {"external_controller": "127.0.0.1:${API_PORT}", "secret": "${SINGBOX_SECRET}"}
  }
}
EOF
	umask 022
	"${BIN_DIR}/sing-box.new" check -c "${SINGBOX_DIR}/config.json" || die "generated sing-box configuration is invalid"
}

# Reuse the secret of an existing installation so both sides keep matching.
load_secret() {
	SINGBOX_SECRET=""
	if [ -f "${SINGBOX_DIR}/config.json" ]; then
		SINGBOX_SECRET="$(sed -n 's/.*"secret"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "${SINGBOX_DIR}/config.json" | head -n 1)"
	fi
	[ -n "$SINGBOX_SECRET" ] || SINGBOX_SECRET="$(random_secret)"
}

# --- TProxy ---------------------------------------------------------------------------

install_tproxy_helper() {
	mkdir -p "$LIB_DIR"
	cat >"${LIB_DIR}/tproxy.sh" <<EOF
#!/bin/sh
# Sends traffic routed to this host for the FakeIP ranges to the sing-box TProxy inbound.
# Installed by the AtomDNS installer; run by the sing-box service on start and stop.
PORT=${TPROXY_PORT}
MARK=${TPROXY_MARK}
TABLE=${TPROXY_TABLE}

stop() {
	nft delete table inet atomdns_tproxy 2>/dev/null
	ip -4 rule del fwmark \$MARK lookup \$TABLE 2>/dev/null
	ip -6 rule del fwmark \$MARK lookup \$TABLE 2>/dev/null
	ip -4 route flush table \$TABLE 2>/dev/null
	ip -6 route flush table \$TABLE 2>/dev/null
	return 0
}

start() {
	stop
	for module in nft_tproxy nft_socket nf_tproxy_ipv4 nf_tproxy_ipv6; do
		modprobe -q \$module 2>/dev/null
	done
	ip -4 rule add fwmark \$MARK lookup \$TABLE
	ip -4 route add local 0.0.0.0/0 dev lo table \$TABLE
	ip -6 rule add fwmark \$MARK lookup \$TABLE 2>/dev/null
	ip -6 route add local ::/0 dev lo table \$TABLE 2>/dev/null
	nft -f - <<NFT
table inet atomdns_tproxy {
	chain prerouting {
		type filter hook prerouting priority mangle; policy accept;
		meta l4proto { tcp, udp } ip daddr ${FAKEIP4} tproxy ip to :\$PORT meta mark set \$MARK accept
		meta l4proto { tcp, udp } ip6 daddr ${FAKEIP6} tproxy ip6 to :\$PORT meta mark set \$MARK accept
	}
}
NFT
}

case "\$1" in
start) start ;;
stop) stop ;;
*) echo "usage: \$0 start|stop" >&2; exit 2 ;;
esac
EOF
	chmod 0755 "${LIB_DIR}/tproxy.sh"
}

# --- services -------------------------------------------------------------------------

install_services() {
	log "installing services"
	if [ "$OS" = alpine ]; then
		cat >/etc/init.d/sing-box <<EOF
#!/sbin/openrc-run
name="sing-box"
description="sing-box proxy for AtomDNS"
supervisor=supervise-daemon
command="${BIN_DIR}/sing-box"
command_args="run -c ${SINGBOX_DIR}/config.json"
pidfile="/run/\${RC_SVCNAME}.pid"
respawn_delay=3
respawn_max=0
output_log="/var/log/sing-box.log"
error_log="/var/log/sing-box.log"

depend() {
	need net
	after firewall
	before atomdns
}

start_pre() {
	"\${command}" check -c ${SINGBOX_DIR}/config.json && ${LIB_DIR}/tproxy.sh start
}

stop_post() {
	${LIB_DIR}/tproxy.sh stop
}
EOF
		cat >/etc/init.d/atomdns <<EOF
#!/sbin/openrc-run
name="AtomDNS"
description="AtomDNS DNS server and WebUI"
supervisor=supervise-daemon
command="${BIN_DIR}/atomdns"
command_args="start -d ${ROOT} -c config_custom.yaml"
pidfile="/run/\${RC_SVCNAME}.pid"
respawn_delay=3
respawn_max=0
output_log="/var/log/atomdns.log"
error_log="/var/log/atomdns.log"

depend() {
	need net
	after firewall sing-box
}
EOF
		chmod 0755 /etc/init.d/sing-box /etc/init.d/atomdns
		rc-update add sing-box default >/dev/null
		rc-update add atomdns default >/dev/null
	else
		cat >/etc/systemd/system/sing-box.service <<EOF
[Unit]
Description=sing-box proxy for AtomDNS
After=network-online.target
Wants=network-online.target
Before=atomdns.service

[Service]
ExecStartPre=${BIN_DIR}/sing-box check -c ${SINGBOX_DIR}/config.json
ExecStartPre=${LIB_DIR}/tproxy.sh start
ExecStart=${BIN_DIR}/sing-box run -c ${SINGBOX_DIR}/config.json
ExecStopPost=${LIB_DIR}/tproxy.sh stop
ExecReload=/bin/kill -HUP \$MAINPID
Restart=always
RestartSec=3
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
EOF
		cat >/etc/systemd/system/atomdns.service <<EOF
[Unit]
Description=AtomDNS DNS server and WebUI
After=network-online.target sing-box.service
Wants=network-online.target

[Service]
ExecStart=${BIN_DIR}/atomdns start -d ${ROOT} -c config_custom.yaml
Restart=always
RestartSec=3
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
EOF
		systemctl daemon-reload
		systemctl enable sing-box atomdns >/dev/null 2>&1
	fi
}

stop_services() {
	if [ "$OS" = alpine ]; then
		for service in atomdns sing-box; do
			[ -x "/etc/init.d/${service}" ] && rc-service "$service" stop >/dev/null 2>&1 || true
		done
	else
		systemctl stop atomdns sing-box >/dev/null 2>&1 || true
	fi
}

start_services() {
	log "starting sing-box and AtomDNS"
	if [ "$OS" = alpine ]; then
		rc-service sing-box start >/dev/null
		rc-service atomdns start >/dev/null
	else
		systemctl start sing-box
		systemctl start atomdns
	fi
	sleep 2
	service_running sing-box || die "sing-box did not start; see /var/log/sing-box.log or journalctl -u sing-box"
	service_running atomdns || die "AtomDNS did not start; see /var/log/atomdns.log or journalctl -u atomdns"
}

# --- main -----------------------------------------------------------------------------

install_packages
check_dns_port
fetch_assets
install_programs
load_secret
install_singbox_config
install_tproxy_helper
stop_services
activate_programs
install_atomdns_config
install_services
"${BIN_DIR}/atomdns" check -d "$ROOT" -c config_custom.yaml >/dev/null || die "AtomDNS configuration check failed"
start_services

ADDR="$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i < NF; i++) if ($i == "src") print $(i + 1)}')"
ADDR="${ADDR:-<this-host>}"
cat <<EOF

AtomDNS $("${BIN_DIR}/atomdns" version) and $("${BIN_DIR}/sing-box" version | head -n 1) are running.

  WebUI:  http://${ADDR}:9099/
  DNS:    ${ADDR}:53

Next steps:
  1. Give this host a fixed IP (static address or a DHCP reservation): clients use
     it as their DNS server and the FakeIP routes point at it.
  2. Add proxy nodes or subscriptions in the WebUI under sing-box.
  3. On the main router, use ${ADDR} as the clients' DNS server and route
     ${FAKEIP4} and ${FAKEIP6} to ${ADDR} (static route or policy routing).
EOF
