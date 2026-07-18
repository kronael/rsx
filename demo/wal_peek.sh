#!/usr/bin/env bash
# Terminal beat for the exchange walkthrough: the traced order sitting in
# the PENGU ME's WAL, read back with rsx-cli. Run from the repo root.
set -euo pipefail

DIM=$'\x1b[38;5;244m'
GRN=$'\x1b[38;5;42m'
CYN=$'\x1b[38;5;80m'
WHT=$'\x1b[38;5;252m'
RST=$'\x1b[0m'

CLI=target/debug/rsx-cli
STRIP='s/len=[0-9]* *//; s/crc=[^ ]* *//; s/ cid=$//'

echo " ${CYN}RSX${RST} ${DIM}·${RST} ${WHT}the same order on disk${RST}" \
  "${DIM}· WAL record = wire record (repr(C), CRC'd)${RST}"
sleep 0.8
echo
echo " ${DIM}\$${RST} rsx-cli wal-dump 10 tmp/wal/pengu --user 1 | tail -4 | sed 's/len.. crc..//'"
"$CLI" wal-dump 10 tmp/wal/pengu --user 1 2>/dev/null \
  | tail -4 | sed "$STRIP" | sed "s/^/ /"
sleep 1.2
echo
echo " ${GRN}fills precede ORDER_DONE${RST}${DIM} — invariant #1, on the tape itself${RST}"
sleep 1.0
echo
echo " ${DIM}\$${RST} rsx-cli wal-dump 10 tmp/wal/pengu --stats"
"$CLI" wal-dump 10 tmp/wal/pengu --stats 2>/dev/null | sed "s/^/ /"
sleep 1.5
