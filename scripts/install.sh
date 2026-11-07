#!/bin/sh
# One-step installer for AtomDNS + a customized proxy backend (DefaWong/atomdns):
# sing-box (DefaWong/sing-box) or mihomo (DefaWong/mihomo).
#
#   curl -fsSL https://raw.githubusercontent.com/DefaWong/atomdns/main/scripts/install.sh | sh
#   wget -qO- https://raw.githubusercontent.com/DefaWong/atomdns/main/scripts/install.sh | sh
#
# Supports Alpine (OpenRC) and Debian/Ubuntu (systemd) on x86_64. Installs AtomDNS and the
# chosen proxy backend, a fresh AtomDNS configuration (backend FakeIP upstream, backend API
# pre-wired) and a backend configuration with a TProxy inbound for the FakeIP ranges. Both
# backends speak the same protocol to AtomDNS, so the WebUI and the rules work the same way.
# Re-running upgrades the programs and the WebUI but keeps existing configuration and the
# installed backend.
#
# Environment overrides:
#   PROXY_BACKEND     sing-box or mihomo; asked on the terminal when unset (default: sing-box)
#   ATOMDNS_RELEASE   release tag to install (default: latest)
#   ATOMDNS_REPO      GitHub repository (default: DefaWong/atomdns)
#   ATOMDNS_ASSETS    local directory holding the release assets instead of downloading
#   ATOMDNS_ROOT      AtomDNS configuration root (default: /cus/atomdns)
#   TPROXY_PORT       backend TProxy port (default: 7895)
set -eu

REPO="${ATOMDNS_REPO:-DefaWong/atomdns}"
RELEASE="${ATOMDNS_RELEASE:-latest}"
ROOT="${ATOMDNS_ROOT:-/cus/atomdns}"
TPROXY_PORT="${TPROXY_PORT:-7895}"
SINGBOX_DIR=/etc/sing-box
SINGBOX_STATE=/var/lib/sing-box
MIHOMO_DIR=/etc/mihomo
MIHOMO_STATE=/var/lib/mihomo
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

# --- proxy backend --------------------------------------------------------------------

# An existing installation keeps its backend; a new one asks on the terminal, which also
# works under `curl ... | sh`.
choose_backend() {
	BACKEND="${PROXY_BACKEND:-}"
	if [ -z "$BACKEND" ]; then
		if [ -f "${MIHOMO_DIR}/config.yaml" ]; then
			BACKEND=mihomo
		elif [ -f "${SINGBOX_DIR}/config.json" ]; then
			BACKEND=sing-box
		elif (: </dev/tty) 2>/dev/null; then
			printf 'Proxy backend: 1) sing-box  2) mihomo [1]: ' >/dev/tty
			read -r answer </dev/tty || answer=""
			case "$answer" in
			2 | mihomo) BACKEND=mihomo ;;
			*) BACKEND=sing-box ;;
			esac
		else
			BACKEND=sing-box
		fi
	fi
	case "$BACKEND" in
	sing-box) PROXY_CONFIG="${SINGBOX_DIR}/config.json" ;;
	mihomo) PROXY_CONFIG="${MIHOMO_DIR}/config.yaml" ;;
	*) die "PROXY_BACKEND must be sing-box or mihomo, not ${BACKEND}" ;;
	esac
	PROXY_BIN="${BIN_DIR}/${BACKEND}"
	PROXY_LOG="/var/log/${BACKEND}.log"
	log "proxy backend: ${BACKEND}"
}

# The backend's check, run and version commands.
proxy_check() {
	if [ "$BACKEND" = mihomo ]; then
		"$1" -t -d "$MIHOMO_STATE" -f "$PROXY_CONFIG"
	else
		"$1" check -c "$PROXY_CONFIG"
	fi
}

proxy_run_args() {
	if [ "$BACKEND" = mihomo ]; then
		echo "-d ${MIHOMO_STATE} -f ${PROXY_CONFIG}"
	else
		echo "run -c ${PROXY_CONFIG}"
	fi
}

proxy_version() {
	if [ "$BACKEND" = mihomo ]; then
		"$1" -v
	else
		"$1" version
	fi
}

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

fetch_assets() {
	# Only the chosen backend is downloaded; SHA256SUMS lists both.
	FILES="atomdns-linux-amd64-musl.tar.gz ${BACKEND}-linux-amd64.tar.gz atomdns-webui.tar.gz config_rust.zip"
	if [ -n "${ATOMDNS_ASSETS:-}" ]; then
		log "using local assets from ${ATOMDNS_ASSETS}"
		for asset in $FILES SHA256SUMS; do
			cp "${ATOMDNS_ASSETS}/${asset}" "$WORK/" || die "missing ${ATOMDNS_ASSETS}/${asset}"
		done
	else
		if [ "$RELEASE" = latest ]; then
			base="https://github.com/${REPO}/releases/latest/download"
		else
			base="https://github.com/${REPO}/releases/download/${RELEASE}"
		fi
		log "downloading ${REPO} ${RELEASE}"
		for asset in $FILES SHA256SUMS; do
			curl -fsSL --retry 3 -o "$WORK/${asset}" "${base}/${asset}" || die "download failed: ${base}/${asset}"
		done
	fi
	for file in $FILES; do
		grep -q "  ${file}\$" "$WORK/SHA256SUMS" || die "SHA256SUMS does not list ${file}"
	done
	(cd "$WORK" && for file in $FILES; do grep "  ${file}\$" SHA256SUMS; done | sha256sum -c - >/dev/null) ||
		die "sha256 verification failed"
}

# --- programs -------------------------------------------------------------------------

install_programs() {
	log "installing programs to ${BIN_DIR}"
	mkdir -p "$WORK/atomdns" "$WORK/proxy"
	tar -xzf "$WORK/atomdns-linux-amd64-musl.tar.gz" -C "$WORK/atomdns"
	tar -xzf "$WORK/${BACKEND}-linux-amd64.tar.gz" -C "$WORK/proxy"
	install -m 0755 "$WORK/atomdns/atomdns" "${BIN_DIR}/atomdns.new"
	install -m 0755 "$WORK/proxy/${BACKEND}" "${PROXY_BIN}.new"
	"${BIN_DIR}/atomdns.new" version >/dev/null || die "the downloaded atomdns does not run here"
	proxy_version "${PROXY_BIN}.new" >/dev/null || die "the downloaded ${BACKEND} does not run here"
}

# Swapped in only while the services are stopped.
activate_programs() {
	mv -f "${BIN_DIR}/atomdns.new" "${BIN_DIR}/atomdns"
	mv -f "${PROXY_BIN}.new" "$PROXY_BIN"
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

	# Both backends serve the Clash API on the same port; the settings key keeps its
	# historical sing-box name.
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
    "secret": "${PROXY_SECRET}"
  }
}
EOF
		umask 022
	fi
	# It holds the backend API secret.
	chmod 0600 "${ROOT}/webinfo/settings.json"
}

# --- sing-box configuration -----------------------------------------------------------

install_singbox_config() {
	mkdir -p "$SINGBOX_DIR" "$SINGBOX_STATE"
	chmod 0700 "$SINGBOX_DIR" "$SINGBOX_STATE"
	if [ -f "$PROXY_CONFIG" ]; then
		log "keeping existing sing-box configuration in ${SINGBOX_DIR}"
		return
	fi
	log "creating sing-box configuration in ${SINGBOX_DIR}"
	umask 077
	cat >"$PROXY_CONFIG" <<EOF
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
    "clash_api": {"external_controller": "127.0.0.1:${API_PORT}", "secret": "${PROXY_SECRET}"}
  }
}
EOF
	umask 022
	proxy_check "${PROXY_BIN}.new" || die "generated sing-box configuration is invalid"
}

# --- mihomo configuration -------------------------------------------------------------

# AtomDNS owns proxies, proxy-providers and the proxy group through the backend API; the
# rest is the fixed AtomDNS topology. AtomDNS sends only the FakeIP domains here.
install_mihomo_config() {
	mkdir -p "$MIHOMO_DIR" "$MIHOMO_STATE"
	chmod 0700 "$MIHOMO_DIR" "$MIHOMO_STATE"
	if [ -f "$PROXY_CONFIG" ]; then
		log "keeping existing mihomo configuration in ${MIHOMO_DIR}"
		return
	fi
	log "creating mihomo configuration in ${MIHOMO_DIR}"
	umask 077
	cat >"$PROXY_CONFIG" <<EOF
log-level: info
mode: rule
ipv6: true
allow-lan: false
find-process-mode: "off"
geo-auto-update: false
external-controller: 127.0.0.1:${API_PORT}
secret: "${PROXY_SECRET}"
profile:
  store-selected: true
  store-fake-ip: true
dns:
  enable: true
  listen: 127.0.0.1:${FAKEIP_DNS_PORT}
  ipv6: true
  enhanced-mode: fake-ip
  fake-ip-range: ${FAKEIP4}
  fake-ip-range6: ${FAKEIP6}
  default-nameserver: [223.5.5.5]
  nameserver: [223.5.5.5]
listeners:
  - {name: atomdns-proxy-in, type: mixed, listen: 127.0.0.1, port: ${MIXED_PORT}}
  - {name: tproxy-in, type: tproxy, listen: "::", port: ${TPROXY_PORT}, udp: true}
proxy-groups:
  - {name: proxy, type: select, proxies: [DIRECT]}
rules:
  - MATCH,proxy
EOF
	umask 022
	proxy_check "${PROXY_BIN}.new" || die "generated mihomo configuration is invalid"
}

install_proxy_config() {
	if [ "$BACKEND" = mihomo ]; then
		install_mihomo_config
	else
		install_singbox_config
	fi
}

# Reuse the secret of an existing installation so both sides keep matching.
load_secret() {
	PROXY_SECRET=""
	if [ -f "$PROXY_CONFIG" ]; then
		if [ "$BACKEND" = mihomo ]; then
			PROXY_SECRET="$(sed -n 's/^secret:[[:space:]]*"\{0,1\}\([^"]*\)"\{0,1\}[[:space:]]*$/\1/p' "$PROXY_CONFIG" | head -n 1)"
		else
			PROXY_SECRET="$(sed -n 's/.*"secret"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$PROXY_CONFIG" | head -n 1)"
		fi
	fi
	[ -n "$PROXY_SECRET" ] || PROXY_SECRET="$(random_secret)"
}

# --- TProxy ---------------------------------------------------------------------------

install_tproxy_helper() {
	mkdir -p "$LIB_DIR"
	cat >"${LIB_DIR}/tproxy.sh" <<EOF
#!/bin/sh
# Sends traffic routed to this host for the FakeIP ranges to the ${BACKEND} TProxy inbound.
# Installed by the AtomDNS installer; run by the ${BACKEND} service on start and stop.
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
	run_args="$(proxy_run_args)"
	if [ "$BACKEND" = mihomo ]; then
		check_cmd="${PROXY_BIN} -t ${run_args}"
		reload_cmd=""
	else
		check_cmd="${PROXY_BIN} check -c ${PROXY_CONFIG}"
		reload_cmd="ExecReload=/bin/kill -HUP \$MAINPID"
	fi
	if [ "$OS" = alpine ]; then
		cat >"/etc/init.d/${BACKEND}" <<EOF
#!/sbin/openrc-run
name="${BACKEND}"
description="${BACKEND} proxy for AtomDNS"
supervisor=supervise-daemon
command="${PROXY_BIN}"
command_args="${run_args}"
pidfile="/run/\${RC_SVCNAME}.pid"
respawn_delay=3
respawn_max=0
output_log="${PROXY_LOG}"
error_log="${PROXY_LOG}"

depend() {
	need net
	after firewall
	before atomdns
}

start_pre() {
	${check_cmd} && ${LIB_DIR}/tproxy.sh start
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
	after firewall ${BACKEND}
}
EOF
		chmod 0755 "/etc/init.d/${BACKEND}" /etc/init.d/atomdns
		rc-update add "$BACKEND" default >/dev/null
		rc-update add atomdns default >/dev/null
	else
		cat >"/etc/systemd/system/${BACKEND}.service" <<EOF
[Unit]
Description=${BACKEND} proxy for AtomDNS
After=network-online.target
Wants=network-online.target
Before=atomdns.service

[Service]
ExecStartPre=${check_cmd}
ExecStartPre=${LIB_DIR}/tproxy.sh start
ExecStart=${PROXY_BIN} ${run_args}
ExecStopPost=${LIB_DIR}/tproxy.sh stop
${reload_cmd}
Restart=always
RestartSec=3
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
EOF
		cat >/etc/systemd/system/atomdns.service <<EOF
[Unit]
Description=AtomDNS DNS server and WebUI
After=network-online.target ${BACKEND}.service
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
		systemctl enable "$BACKEND" atomdns >/dev/null 2>&1
	fi
}

stop_services() {
	if [ "$OS" = alpine ]; then
		for service in atomdns "$BACKEND"; do
			[ -x "/etc/init.d/${service}" ] && rc-service "$service" stop >/dev/null 2>&1 || true
		done
	else
		systemctl stop atomdns "$BACKEND" >/dev/null 2>&1 || true
	fi
}

start_services() {
	log "starting ${BACKEND} and AtomDNS"
	if [ "$OS" = alpine ]; then
		rc-service "$BACKEND" start >/dev/null
		rc-service atomdns start >/dev/null
	else
		systemctl start "$BACKEND"
		systemctl start atomdns
	fi
	sleep 2
	service_running "$BACKEND" || die "${BACKEND} did not start; see ${PROXY_LOG} or journalctl -u ${BACKEND}"
	service_running atomdns || die "AtomDNS did not start; see /var/log/atomdns.log or journalctl -u atomdns"
}

# --- main -----------------------------------------------------------------------------

choose_backend
install_packages
check_dns_port
fetch_assets
install_programs
load_secret
install_proxy_config
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

AtomDNS $("${BIN_DIR}/atomdns" version) and $(proxy_version "$PROXY_BIN" | head -n 1) are running.

  WebUI:  http://${ADDR}:9099/
  DNS:    ${ADDR}:53

Next steps:
  1. Give this host a fixed IP (static address or a DHCP reservation): clients use
     it as their DNS server and the FakeIP routes point at it.
  2. Add proxy nodes or subscriptions in the WebUI under 代理.
  3. On the main router, use ${ADDR} as the clients' DNS server and route
     ${FAKEIP4} and ${FAKEIP6} to ${ADDR} (static route or policy routing).
EOF
