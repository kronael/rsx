#!/usr/bin/env bash
# Mint a webd chat token for the `main` agent and print the RSX_TERM_ASSIST URL
# the terminal dials. Works around the split-topology quirk (arizuko token issue
# writes messages.db; webd reads routd.db) by copying the row across.
# Prereq: the rsx stack is up (deploy-local.sh) and the credential is in runed.env.
set -euo pipefail
ARIZUKO_SRC=/home/onvos/app/arizuko
export PREFIX=/home/onvos/.arizuko
INST=rsx
DATA="$PREFIX/data/arizuko_${INST}"
U1000=(sudo -u '#1000' env "PREFIX=$PREFIX" HOME=/home/onvos)

# mint (writes messages.db) — capture the raw token
TOKEN=$("${U1000[@]}" "$ARIZUKO_SRC/arizuko" token "$INST" issue chat main | awk '/^token:/{print $2}')
[ -n "$TOKEN" ] || { echo "mint failed" >&2; exit 1; }

# copy route_tokens rows into routd.db (where webd resolves). Busy-timeout so a
# concurrent routd write doesn't abort the copy.
"${U1000[@]}" python3 - "$DATA" <<'PY'
import sqlite3, sys
data = sys.argv[1]
src = sqlite3.connect(f"{data}/store/messages.db", timeout=10)
dst = sqlite3.connect(f"{data}/store/routd.db", timeout=10)
dst.execute("PRAGMA busy_timeout=10000")
rows = src.execute("SELECT token_hash, jid, owner_folder, created_at FROM route_tokens").fetchall()
dst.executemany(
    "INSERT OR IGNORE INTO route_tokens(token_hash, jid, owner_folder, created_at) VALUES (?,?,?,?)",
    rows)
dst.commit()
print(f"copied {len(rows)} route_token row(s) → routd.db", file=sys.stderr)
PY

echo "RSX_TERM_ASSIST=http://localhost:8095/chat/$TOKEN"
echo "# run the terminal live, e.g.:"
echo "#   RSX_TERM_ASSIST=http://localhost:8095/chat/$TOKEN RSX_TERM_NEWS=1 go run ./rsx-term"
