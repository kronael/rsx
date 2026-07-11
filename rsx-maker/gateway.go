package main

import (
	"context"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"time"

	"github.com/coder/websocket"
)

// dialGateway opens an authenticated gateway WS connection. A fresh
// JWT (with a fresh jti) is minted per dial so the gateway's replay
// tracker never rejects a reconnect.
func dialGateway(ctx context.Context, cfg Config) (*websocket.Conn, error) {
	token, err := mintJWT(cfg.JWTSecret, cfg.UserID)
	if err != nil {
		return nil, err
	}
	header := http.Header{}
	header.Set("Authorization", "Bearer "+token)
	conn, _, err := websocket.Dial(ctx, cfg.GatewayURL, &websocket.DialOptions{
		HTTPHeader: header,
	})
	if err != nil {
		return nil, err
	}
	return conn, nil
}

// sendNewOrder submits a limit order over an open gateway connection.
// Frame: {"N":[symbol_id, side, px, qty, cid, tif]} — matching the
// Python maker (reduce_only/post_only default false when omitted).
func sendNewOrder(ctx context.Context, conn *websocket.Conn, sym uint32, side int, px, qty int64, cid string, tif int) error {
	frame := map[string][]any{"N": {sym, side, px, qty, cid, tif}}
	return writeJSON(ctx, conn, frame)
}

// sendCancel cancels a resting order by client order id.
// Frame: {"C":[cid]} where cid is the 20-char client order id.
func sendCancel(ctx context.Context, conn *websocket.Conn, cid string) error {
	frame := map[string][]any{"C": {cid}}
	return writeJSON(ctx, conn, frame)
}

func writeJSON(ctx context.Context, conn *websocket.Conn, v any) error {
	data, err := json.Marshal(v)
	if err != nil {
		return err
	}
	return conn.Write(ctx, websocket.MessageText, data)
}

// mintJWT builds an HS256 gateway token. Claims match the contract the
// gateway enforces (rsx-gateway/src/jwt.rs): aud=rsx-gateway,
// iss=rsx-auth, HS256 over the shared secret, plus a fresh jti so the
// JtiTracker accepts each handshake.
func mintJWT(secret string, userID uint32) (string, error) {
	if secret == "" {
		return "", fmt.Errorf("RSX_GW_JWT_SECRET not set")
	}
	header := map[string]string{"alg": "HS256", "typ": "JWT"}
	claims := map[string]any{
		"sub":     fmt.Sprintf("maker:%d", userID),
		"user_id": userID,
		"aud":     "rsx-gateway",
		"iss":     "rsx-auth",
		"exp":     time.Now().Add(time.Hour).Unix(),
		"jti":     randomHex(16),
	}
	headerJSON, err := json.Marshal(header)
	if err != nil {
		return "", err
	}
	claimsJSON, err := json.Marshal(claims)
	if err != nil {
		return "", err
	}
	enc := base64.RawURLEncoding
	signingInput := enc.EncodeToString(headerJSON) + "." + enc.EncodeToString(claimsJSON)
	mac := hmac.New(sha256.New, []byte(secret))
	mac.Write([]byte(signingInput))
	sig := enc.EncodeToString(mac.Sum(nil))
	return signingInput + "." + sig, nil
}

// randomHex returns n random bytes as a hex string, for a unique jti.
func randomHex(n int) string {
	buf := make([]byte, n)
	if _, err := rand.Read(buf); err != nil {
		// crypto/rand failure is unrecoverable; fall back to time so
		// the maker still runs (jti uniqueness degrades, exp still
		// bounds replay).
		return fmt.Sprintf("%x", time.Now().UnixNano())
	}
	return hex.EncodeToString(buf)
}
