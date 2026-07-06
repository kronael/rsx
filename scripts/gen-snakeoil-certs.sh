#!/bin/sh
# gen-snakeoil-certs.sh — self-signed certs for RSX replication (TCP).
#
# Replication (rsx-cast ReplicationService/ReplicationConsumer) is
# TLS-mandatory. This writes a throwaway self-signed cert+key the
# replication server presents and the consumer trusts as its CA
# (ca.pem is a copy of cert.pem: single-box self-trust). Real
# deployments replace these with proper certs and point
# RSX_REPL_CERT_PATH/KEY_PATH/CA_PATH at them.
#
# The casting/UDP path stays plaintext by design (trusted LAN,
# spec 4-cast §10.4) — these certs never touch it.
#
# Run from the repo root:  sh scripts/gen-snakeoil-certs.sh [--force]
set -eu

CERT_DIR="${RSX_REPL_CERT_DIR:-./certs}"

FORCE=0
for arg in "$@"; do
    case "$arg" in
        --force) FORCE=1 ;;
        *)
            echo "usage: gen-snakeoil-certs.sh [--force]" >&2
            exit 2
            ;;
    esac
done

# Guard: never mkdir/write into an empty or root path.
case "$CERT_DIR" in
    "" | "/" | "//")
        echo "Refusing to operate on CERT_DIR='$CERT_DIR'" >&2
        exit 1
        ;;
esac

CERT="$CERT_DIR/cert.pem"
KEY="$CERT_DIR/key.pem"
CA="$CERT_DIR/ca.pem"

log() { echo "$(date '+%b %e %H:%M:%S') info gen-snakeoil-certs: $1"; }

if [ "$FORCE" -eq 0 ] && [ -f "$CERT" ] && [ -f "$KEY" ] \
    && [ -f "$CA" ]; then
    log "certs already present in $CERT_DIR (--force to regenerate)"
    exit 0
fi

mkdir -p "$CERT_DIR"

openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$KEY" -out "$CERT" \
    -days 3650 -subj "/CN=localhost" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
    >/dev/null 2>&1

cp "$CERT" "$CA"
chmod 600 "$KEY"

log "wrote cert.pem key.pem ca.pem to $CERT_DIR \
(CN=localhost SAN=localhost,127.0.0.1)"
