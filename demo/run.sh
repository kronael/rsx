#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

cleanup() {
  ./rsx-playground/playground stop-all >/dev/null 2>&1 || true
  ./rsx-playground/playground stop >/dev/null 2>&1 || true
}
trap cleanup EXIT

say() {
  printf '\n\033[1;36m%s\033[0m\n' "$1"
}

run() {
  printf '\033[2m$ %s\033[0m\n' "$*"
  "$@"
}

clear
say "RSX local demo"
printf 'one command boots the exchange, maker, live book, and terminal\n'

say "1. Boot"
run make demo

say "2. Running processes"
run ./rsx-playground/playground ps

say "3. Live market"
python3 - <<'PY'
import json, re, urllib.request

base = "http://localhost:49171"
maker = json.load(urllib.request.urlopen(base + "/api/maker/status"))
book = urllib.request.urlopen(base + "/x/book-stats").read().decode()
pengu = re.search(
    r"<tr><td>PENGU</td><td[^>]*>([^<]+)</td><td[^>]*>([^<]+)</td>"
    r"<td[^>]*>([^<]+)</td><td[^>]*>([^<]+)</td>",
    book,
)
print(f"maker: running={maker['running']} pid={maker['pid']} levels={maker['levels']}")
if pengu:
    bid, ask, spread, orders = pengu.groups()
    print(f"PENGU: bid={bid} ask={ask} spread={spread} orders={orders}")
else:
    raise SystemExit("PENGU book missing")
PY

say "4. Embedded terminal"
rsx-playground/.venv/bin/python - <<'PY'
import asyncio, websockets

async def main():
    async with websockets.connect("ws://localhost:49171/ws/terminal") as ws:
        buf = ""
        for _ in range(80):
            try:
                buf += await asyncio.wait_for(ws.recv(), timeout=0.5)
            except asyncio.TimeoutError:
                continue
            if "PENGU-PERP" in buf:
                print("terminal: live rsx-term rendered PENGU-PERP")
                return
        raise SystemExit("terminal did not render")

asyncio.run(main())
PY

say "5. Stop"
run ./rsx-playground/playground stop-all
run ./rsx-playground/playground stop

say "Demo complete"
