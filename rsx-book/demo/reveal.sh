#!/usr/bin/env bash
# rsx-book match-latency reveal — portrait terminal demo.
# Real numbers from reports/20260704_book-bench.md (Ryzen 9 5950X, 1 core).
# Regenerate the GIF: see rsx-book/demo/CLAUDE.md.
set -u
p(){ printf '%b\n' "$1"; sleep "${2:-0.45}"; }
G=$'\e[32m'; C=$'\e[36m'; Y=$'\e[33m'; B=$'\e[1m'; D=$'\e[2m'; R=$'\e[0m'
clear
p ""
p "  ${B}${C}rsx-book${R}"
p "  ${D}orderbook matching engine${R}" 0.9
p ""
p "  ${D}how fast is one match?${R}" 1.0
p "  ${D}does book depth slow it down?${R}" 1.3
p ""
p "  ${D}resting orders   match latency${R}"
p "  ${D}──────────────   ─────────────${R}" 0.6
p "  ${C}       100,000${R}     ${G}${B}64 ns${R}" 0.9
p "  ${C}     1,000,000${R}     ${G}${B}66 ns${R}" 0.9
p "  ${C}    10,000,000${R}     ${G}${B}65 ns${R}" 1.4
p ""
p "  ${Y}${B}10 million orders deep.${R}" 0.9
p "  ${Y}${B}still 65 nanoseconds.${R}" 1.4
p ""
p "  ${B}O(1).${R} ${D}depth doesn't matter.${R}" 1.6
p ""
p "  ${D}slab alloc     556 ps${R}" 0.5
p "  ${D}price -> index  1.9 ns${R}" 0.5
p "  ${D}best match       28 ns${R}" 1.0
p ""
p "  ${D}─────────────────────────${R}"
p "  ${D}1 core - Ryzen 9 5950X${R}"
p "  ${D}criterion - lab microbench${R}" 2.0
p ""
