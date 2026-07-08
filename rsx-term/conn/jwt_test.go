package conn

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"strconv"
	"strings"
	"testing"
	"time"
)

func TestMintJWTShapeAndSignature(t *testing.T) {
	secret := "test-secret-at-least-32-bytes-long!"
	before := time.Now()
	token := mintJWT(7, secret)
	after := time.Now()

	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		t.Fatalf("token has %d parts, want 3 (header.claims.sig)", len(parts))
	}

	headerJSON, err := base64.RawURLEncoding.DecodeString(parts[0])
	if err != nil {
		t.Fatalf("decode header: %v", err)
	}
	if string(headerJSON) != jwtHeader {
		t.Fatalf("header = %s, want %s", headerJSON, jwtHeader)
	}

	claimsJSON, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		t.Fatalf("decode claims: %v", err)
	}
	var claims jwtClaims
	if err := json.Unmarshal(claimsJSON, &claims); err != nil {
		t.Fatalf("unmarshal claims: %v", err)
	}
	if claims.Sub != "rsx-term:7" {
		t.Fatalf("sub = %q, want rsx-term:7", claims.Sub)
	}
	if claims.UserID != 7 {
		t.Fatalf("user_id = %d, want 7", claims.UserID)
	}
	if claims.Aud != "rsx-gateway" {
		t.Fatalf("aud = %q, want rsx-gateway", claims.Aud)
	}
	if claims.Iss != "rsx-auth" {
		t.Fatalf("iss = %q, want rsx-auth", claims.Iss)
	}
	wantExp := before.Add(jwtTTL).Unix()
	maxExp := after.Add(jwtTTL).Unix()
	if claims.Exp < wantExp || claims.Exp > maxExp {
		t.Fatalf("exp = %d, want in [%d, %d]", claims.Exp, wantExp, maxExp)
	}
	if !strings.HasPrefix(claims.Jti, "rsx-term-7-") {
		t.Fatalf("jti = %q, want rsx-term-7- prefix", claims.Jti)
	}

	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(parts[0] + "." + parts[1]))
	wantSig := base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	if parts[2] != wantSig {
		t.Fatalf("signature = %q, want %q", parts[2], wantSig)
	}
}

func TestMintJWTFreshJtiPerCall(t *testing.T) {
	secret := "test-secret-at-least-32-bytes-long!"
	a := mintJWT(1, secret)
	b := mintJWT(1, secret)
	if a == b {
		t.Fatalf("two mintJWT calls produced identical tokens (jti must be fresh per call)")
	}
}

// TestMintJWTVerifiesWithoutSecret sanity-checks the claims JSON round-trips
// through strconv-parseable exp so a caller inspecting a token off the wire
// (as the gateway does) sees plain integers, not strings.
func TestMintJWTExpIsNumeric(t *testing.T) {
	token := mintJWT(1, "s")
	parts := strings.Split(token, ".")
	claimsJSON, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		t.Fatalf("decode claims: %v", err)
	}
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(claimsJSON, &raw); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if _, err := strconv.ParseInt(string(raw["exp"]), 10, 64); err != nil {
		t.Fatalf("exp not numeric: %s", raw["exp"])
	}
}
