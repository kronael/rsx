#!/usr/bin/env bash
# RSX single-machine production deploy. Run ON the target host
# (e.g. rsx.krons.cx) as root, from the repo root:
#
#   sudo ./deploy/deploy.sh            # dry-run: print the plan, touch nothing
#   sudo ./deploy/deploy.sh --apply    # build, install, enable, verify
#
# Idempotent: re-running --apply rebuilds, re-stages, and restarts.
# It NEVER sshes and NEVER pushes — this script only ever operates on
# localhost. External steps (DNS, TLS cert, git push, offload cron)
# are the founder's, and are listed in deploy/README.md.
set -euo pipefail

# ── fixed working directory: must run from the repo root ──
if [[ ! -f Cargo.toml || ! -d deploy/systemd ]]; then
  echo "Run from the repo root: sudo ./deploy/deploy.sh [--apply]" >&2
  exit 1
fi

PREFIX="${PREFIX:-/srv}"
DATABASE_URL="${DATABASE_URL:-postgres://rsx@127.0.0.1:5432/rsx}"
BIN_DIR=/opt/rsx/bin
ENV_DIR=/opt/rsx/env
SPEC_DIR=/opt/rsx/specs
DATA_DIR="$PREFIX/data/rsx"
ARCHIVE_DIR="$DATA_DIR/archive"
UNIT_DIR=/etc/systemd/system

APPLY=0
SKIP_BUILD=0
EXPECT_HOST="${RSX_DEPLOY_HOST:-}"
for a in "$@"; do
  case "$a" in
    --apply) APPLY=1 ;;
    --dry-run) APPLY=0 ;;
    --skip-build) SKIP_BUILD=1 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "unknown arg: $a" >&2; exit 1 ;;
  esac
done

# Instance lists (single-machine prod: BTC/ETH/SOL majors, shard 0).
SYMBOLS=(btc eth sol)
ME_UNITS=(rsx-me@btc rsx-me@eth rsx-me@sol)
REC_UNITS=(rsx-recorder@btc rsx-recorder@eth rsx-recorder@sol)
ALL_UNITS=(
  "${ME_UNITS[@]}" rsx-mark rsx-risk@0
  rsx-gateway rsx-marketdata "${REC_UNITS[@]}"
)
# health addr per unit, in the order verified after start.
HEALTH=(
  127.0.0.1:9801 127.0.0.1:9802 127.0.0.1:9803  # me btc/eth/sol
  127.0.0.1:9830                                  # mark
  127.0.0.1:9810                                  # risk-0
  127.0.0.1:9820                                  # gateway
  127.0.0.1:9840                                  # marketdata
  127.0.0.1:9851 127.0.0.1:9852 127.0.0.1:9853    # recorder btc/eth/sol
)

say() { printf '%s %s\n' "$(date '+%b %d %H:%M:%S') INFO deploy:" "$*"; }
run() { if [[ $APPLY -eq 1 ]]; then "$@"; else echo "  would: $*"; fi; }

if [[ $APPLY -eq 0 ]]; then
  say "DRY RUN — no changes. Re-run with --apply to execute."
fi

# ── guard: confirm we are on the intended host ──
if [[ $APPLY -eq 1 ]]; then
  if [[ $EUID -ne 0 ]]; then
    echo "Must run as root for --apply (sudo)." >&2; exit 1
  fi
  here="$(hostname -f 2>/dev/null || hostname)"
  if [[ -n "$EXPECT_HOST" && "$here" != "$EXPECT_HOST" ]]; then
    echo "Refusing: hostname '$here' != RSX_DEPLOY_HOST '$EXPECT_HOST'." >&2
    echo "Set RSX_DEPLOY_HOST to this box's FQDN to confirm intent." >&2
    exit 1
  fi
  say "applying on host: $here (prefix $PREFIX)"
fi

# ── 1. build release binaries ──
if [[ $SKIP_BUILD -eq 0 ]]; then
  say "building release workspace"
  run cargo build --release --workspace
fi

# ── 2. system user + directories ──
say "ensuring rsx user and data layout"
if [[ $APPLY -eq 1 ]] && ! id rsx >/dev/null 2>&1; then
  useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin rsx
fi
run install -d -o rsx -g rsx -m 0750 "$BIN_DIR" "$ENV_DIR" "$SPEC_DIR/2"
run install -d -o rsx -g rsx -m 0750 \
  "$DATA_DIR" "$DATA_DIR/wal" "$DATA_DIR/wal/mark" \
  "$DATA_DIR/snapshot" "$DATA_DIR/log"
for s in "${SYMBOLS[@]}"; do
  run install -d -o rsx -g rsx -m 0750 "$DATA_DIR/wal/$s"
done
# The archive is unbounded by design — mount a dedicated volume here
# BEFORE first run (deploy/README.md). We only create the dir.
run install -d -o rsx -g rsx -m 0750 "$ARCHIVE_DIR"

# ── 2b. replication TLS certs (mandatory; casting/UDP stays plaintext) ──
# Replication is TLS-only. Provision snakeoil certs so the cluster boots;
# replace with real certs in prod (deploy/README.md). Idempotent: the
# script skips if certs already exist.
CERT_DIR="$DATA_DIR/certs"
run install -d -o rsx -g rsx -m 0750 "$CERT_DIR"
if [[ $APPLY -eq 1 ]]; then
  RSX_REPL_CERT_DIR="$CERT_DIR" sh scripts/gen-snakeoil-certs.sh
  chown -R rsx:rsx "$CERT_DIR"
  say "replication certs in $CERT_DIR (snakeoil — replace with real certs)"
else
  echo "  would: generate snakeoil replication certs into $CERT_DIR"
fi

# ── 3. install binaries ──
say "installing binaries to $BIN_DIR"
for b in rsx-matching rsx-risk rsx-gateway rsx-marketdata rsx-mark rsx-recorder; do
  run install -m 0755 "target/release/$b" "$BIN_DIR/$b"
done
run install -m 0644 specs/2/9-deploy.md "$SPEC_DIR/2/9-deploy.md"

# ── 4. stage env files (substitute PREFIX + DATABASE_URL only) ──
say "staging env files to $ENV_DIR"
export PREFIX DATABASE_URL
for f in deploy/env/*.env; do
  [[ -e "$f" ]] || continue
  dst="$ENV_DIR/$(basename "$f")"
  if [[ $APPLY -eq 1 ]]; then
    envsubst '${PREFIX} ${DATABASE_URL}' < "$f" > "$dst"
    chown rsx:rsx "$dst"; chmod 0640 "$dst"
  else
    echo "  would: envsubst $f -> $dst (0640 rsx)"
  fi
done

# ── 5. secret file must exist and be real ──
secret="$ENV_DIR/secret.env"
if [[ $APPLY -eq 1 ]]; then
  if [[ ! -f "$secret" ]]; then
    echo "Missing $secret. Create it from deploy/env/secret.env.example:" >&2
    echo "  install -m 0400 -o rsx -g rsx <filled-secret> $secret" >&2
    exit 1
  fi
  if grep -q 'REPLACE_WITH_openssl_rand_hex_32' "$secret"; then
    echo "$secret still holds the placeholder — set a real JWT secret." >&2
    exit 1
  fi
  chown rsx:rsx "$secret"; chmod 0400 "$secret"
else
  echo "  check: $secret present, mode 0400, no placeholder"
fi

# ── 6. sysctl tuning (casting/UDP buffers) ──
say "applying sysctl tuning"
run install -m 0644 deploy/sysctl/99-rsx.conf /etc/sysctl.d/99-rsx.conf
run sysctl --system

# ── 7. install units + enable ──
say "installing systemd units"
run install -m 0644 deploy/systemd/rsx-me@.service "$UNIT_DIR/"
run install -m 0644 deploy/systemd/rsx-risk@.service "$UNIT_DIR/"
run install -m 0644 deploy/systemd/rsx-recorder@.service "$UNIT_DIR/"
for u in rsx-gateway rsx-marketdata rsx-mark; do
  run install -m 0644 "deploy/systemd/$u.service" "$UNIT_DIR/"
done
run install -m 0644 deploy/systemd/rsx.target "$UNIT_DIR/"
run systemctl daemon-reload

say "enabling units (Postgres must already be up as rsx-postgres.service)"
for u in "${ALL_UNITS[@]}"; do
  run systemctl enable "$u.service"
done
run systemctl enable rsx.target
run systemctl start rsx.target

# ── 8. verify health ──
if [[ $APPLY -eq 1 ]]; then
  say "verifying /health (up to 30s per endpoint)"
  fail=0
  for h in "${HEALTH[@]}"; do
    ok=0
    for _ in $(seq 1 30); do
      if curl -fsS "http://$h/health" >/dev/null 2>&1; then ok=1; break; fi
      sleep 1
    done
    if [[ $ok -eq 1 ]]; then say "  ok   $h/health"; else
      echo "  FAIL $h/health" >&2; fail=1; fi
  done
  if [[ $fail -ne 0 ]]; then
    echo "One or more health checks failed — see: journalctl -u 'rsx-*'" >&2
    exit 1
  fi
  say "cluster up. Gateway WS on 127.0.0.1:8080 — front it with nginx TLS."
else
  say "dry run complete. Re-run with --apply on the target as root."
fi
