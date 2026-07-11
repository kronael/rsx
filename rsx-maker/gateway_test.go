package main

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"strings"
	"testing"
)

func TestMintJWTVerifies(t *testing.T) {
	secret := "test-secret"
	token, err := mintJWT(secret, 99)
	if err != nil {
		t.Fatalf("mintJWT: %v", err)
	}
	parts := strings.Split(token, ".")
	if len(parts) != 3 {
		t.Fatalf("token has %d parts, want 3", len(parts))
	}

	// Signature must verify against the secret.
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(parts[0] + "." + parts[1]))
	want := base64.RawURLEncoding.EncodeToString(mac.Sum(nil))
	if parts[2] != want {
		t.Errorf("signature mismatch: got %s want %s", parts[2], want)
	}

	// Claims must carry the contract the gateway enforces.
	claimsJSON, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		t.Fatalf("decode claims: %v", err)
	}
	var claims map[string]any
	if err := json.Unmarshal(claimsJSON, &claims); err != nil {
		t.Fatalf("unmarshal claims: %v", err)
	}
	if claims["aud"] != "rsx-gateway" {
		t.Errorf("aud = %v, want rsx-gateway", claims["aud"])
	}
	if claims["iss"] != "rsx-auth" {
		t.Errorf("iss = %v, want rsx-auth", claims["iss"])
	}
	if claims["user_id"] != float64(99) {
		t.Errorf("user_id = %v, want 99", claims["user_id"])
	}
	if claims["jti"] == nil || claims["jti"] == "" {
		t.Error("jti missing")
	}
}

func TestMintJWTFreshJTI(t *testing.T) {
	a, _ := mintJWT("s", 1)
	b, _ := mintJWT("s", 1)
	if a == b {
		t.Error("consecutive tokens identical; jti not fresh")
	}
}

func TestMintJWTRequiresSecret(t *testing.T) {
	if _, err := mintJWT("", 1); err == nil {
		t.Error("expected error for empty secret")
	}
}
