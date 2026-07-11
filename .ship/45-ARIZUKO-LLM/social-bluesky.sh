#!/usr/bin/env bash
# Phase 3 — put the SAME RSX agent on Bluesky (social through arizuko).
# The agent folder `main` (persona + per-thread memory) is already live for the
# terminal; this attaches a Bluesky identity to it via ONE route row + the bskyd
# adapter. A mention/DM on Bluesky then becomes a routed turn to the same agent.
#
# Founder-supplied (NOT on the box — unlike the Claude token in ~/.bashrc):
#   BLUESKY_IDENTIFIER  — the bot handle, e.g. rsxdesk.bsky.social
#   BLUESKY_PASSWORD    — an app password (Settings → App Passwords), NOT the main pw
#   BLUESKY_DID         — the account DID (did:plc:…); resolve from the handle:
#     curl -s "https://bsky.social/xrpc/com.atproto.identity.resolveHandle?handle=<handle>"
#
# Usage:
#   BLUESKY_IDENTIFIER=rsxdesk.bsky.social BLUESKY_PASSWORD=xxxx-xxxx-xxxx-xxxx \
#   BLUESKY_DID=did:plc:abc123 ./social-bluesky.sh
set -euo pipefail
: "${BLUESKY_IDENTIFIER:?set BLUESKY_IDENTIFIER}"
: "${BLUESKY_PASSWORD:?set BLUESKY_PASSWORD}"
: "${BLUESKY_DID:?set BLUESKY_DID (did:plc:…)}"

ARIZUKO_SRC=/home/onvos/app/arizuko
export PREFIX=/home/onvos/.arizuko
INST=rsx
DATA="$PREFIX/data/arizuko_${INST}"
COMPOSE="$DATA/docker-compose.yml"
U1000=(sudo -u '#1000' env "PREFIX=$PREFIX" HOME=/home/onvos)

# 1. enable the bskyd adapter for this instance (compose renders services/*.toml)
"${U1000[@]}" install -m 644 "$ARIZUKO_SRC/template/services/bskyd.toml" "$DATA/services/bskyd.toml"

# 2. creds into .env (bskyd reads BLUESKY_IDENTIFIER/PASSWORD)
"${U1000[@]}" bash -c "cat >> '$DATA/.env'" <<EOF

# --- Bluesky adapter (Phase 3 social) ---
BLUESKY_IDENTIFIER=$BLUESKY_IDENTIFIER
BLUESKY_PASSWORD=$BLUESKY_PASSWORD
EOF

# 3. regenerate + bring up (bskyd self-registers with routd at boot)
cd "$ARIZUKO_SRC"
"${U1000[@]}" ./arizuko generate "$INST"
# re-append the agent credential runed needs (generate rewrites runed.env):
TOKEN=$(grep -E '^\s*export\s+CLAUDE_CODE_OAUTH_TOKEN=' ~/.bashrc | grep -v '^\s*#' | tail -1 | sed -E 's/^\s*export\s+CLAUDE_CODE_OAUTH_TOKEN=//; s/^"//; s/"$//')
printf 'CLAUDE_CODE_OAUTH_TOKEN=%s\n' "$TOKEN" | "${U1000[@]}" tee -a "$DATA/env/runed.env" >/dev/null
sudo docker compose -f "$COMPOSE" up -d --remove-orphans

# 4. attach the Bluesky identity to the SAME `main` agent folder (one route row).
#    Threads stay separate from the terminal by topic: Bluesky topics are post
#    AT-URIs, terminal topics are t-… — no cross-bleed, same persona + memory.
cd /tmp
"${U1000[@]}" "$ARIZUKO_SRC/arizuko" group "$INST" add "bluesky:user/$BLUESKY_DID" main

echo "done — mention @$BLUESKY_IDENTIFIER on Bluesky; bskyd polls notifications"
echo "(~10s) and routes each to the rsx 'main' agent, same persona + memory."
