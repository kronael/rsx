#!/bin/sh
# gen-snakeoil-certs.sh — self-signed certs for RSX replication (TCP).
#
# Replication (rsx-cast ReplicationService/ReplicationConsumer) is
# TLS-mandatory. This writes a throwaway CA + a server leaf the
# replication server presents and the consumer trusts:
#
#   ca.pem   — self-signed CA (CA:TRUE), the consumer's trust anchor
#   cert.pem — server leaf (CA:FALSE, EKU serverAuth+clientAuth,
#              SAN localhost/127.0.0.1) signed by that CA
#   key.pem  — the server leaf's private key
#
# The leaf MUST be a distinct end-entity cert: rustls/webpki reject a
# CA:TRUE cert presented as a leaf with `CaUsedAsEndEntity`, so the
# old "ca.pem is a copy of cert.pem" scheme could not complete a
# handshake. Real deployments replace these with proper certs and
# point RSX_REPL_CERT_PATH/KEY_PATH/CA_PATH at them.
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

# Throwaway CA key + CSR live in a temp dir: the runtime needs only
# ca.pem / cert.pem / key.pem, not the CA key (single-box snakeoil).
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM
CA_KEY="$TMP/ca-key.pem"
CSR="$TMP/server.csr"
EXT="$TMP/leaf.ext"

# 1. Self-signed CA (CA:TRUE) — the consumer's trust anchor.
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$CA_KEY" -out "$CA" \
    -days 3650 -subj "/CN=RSX Replication Snakeoil CA" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    >/dev/null 2>&1

# 2. Server leaf key + CSR (CN=localhost).
openssl req -newkey rsa:2048 -nodes \
    -keyout "$KEY" -out "$CSR" \
    -subj "/CN=localhost" \
    >/dev/null 2>&1

# 3. Sign the leaf with the CA: end-entity (CA:FALSE), serverAuth so
#    webpki accepts it for ServerName validation, SAN for both the
#    localhost DNS name and the 127.0.0.1 IP the peers dial.
cat > "$EXT" <<'EOF'
basicConstraints=critical,CA:FALSE
keyUsage=critical,digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth,clientAuth
subjectAltName=DNS:localhost,IP:127.0.0.1
EOF

openssl x509 -req -in "$CSR" \
    -CA "$CA" -CAkey "$CA_KEY" -CAcreateserial \
    -days 3650 -out "$CERT" \
    -extfile "$EXT" \
    >/dev/null 2>&1

chmod 600 "$KEY"

log "wrote cert.pem key.pem ca.pem to $CERT_DIR \
(leaf CN=localhost SAN=localhost,127.0.0.1 signed by snakeoil CA)"
