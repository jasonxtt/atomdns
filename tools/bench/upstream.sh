#!/usr/bin/env bash
# Starts or stops the fake upstream (CoreDNS + Corefile.template) as the transient
# systemd unit `atomdns-bench-upstream`, pinned to the given CPUs.
#   upstream.sh start <coredns binary> <listen address> <port> <cpus, e.g. "2 3">
#   upstream.sh stop
set -euo pipefail
UNIT=atomdns-bench-upstream
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
case "${1:-}" in
  start)
    BIN="${2:?coredns binary}" LISTEN="${3:?listen address}" PORT="${4:?port}" CPUS="${5:?cpus}"
    RUN="${HOME}/bench/upstream"
    mkdir -p "${RUN}"
    sed -e "s/@LISTEN@/${LISTEN}/" -e "s/@PORT@/${PORT}/" "${DIR}/Corefile.template" > "${RUN}/Corefile"
    systemctl stop "${UNIT}" 2>/dev/null || true
    systemctl reset-failed "${UNIT}" 2>/dev/null || true
    systemd-run --unit "${UNIT}" -p "CPUAffinity=${CPUS}" -p LimitNOFILE=65536 \
      --working-directory "${RUN}" "${BIN}" -conf "${RUN}/Corefile" -quiet
    sleep 1
    systemctl is-active "${UNIT}"
    ;;
  stop)
    systemctl stop "${UNIT}" 2>/dev/null || true
    systemctl reset-failed "${UNIT}" 2>/dev/null || true
    ;;
  *) echo "usage: upstream.sh start <coredns> <listen> <port> <cpus> | stop" >&2; exit 2 ;;
esac
