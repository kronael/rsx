package conn

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"time"
)

// jwtHeader is the fixed HS256 JWT header — always the same two fields, so
// it is a constant rather than a marshaled struct.
const jwtHeader = `{"alg":"HS256","typ":"JWT"}`

// jwtTTL is how long a minted token is valid. The gateway rejects an
// expired token; a fresh token is minted once per connection, so this only
// needs to outlive one terminal session comfortably.
const jwtTTL = time.Hour

// jwtClaims mirrors rsx-tui/src/quic.rs mint_jwt's Claims struct field-for-
// field so the gateway's auth extraction is identical regardless of which
// client minted the token.
type jwtClaims struct {
	Sub    string `json:"sub"`
	UserID uint32 `json:"user_id"`
	Aud    string `json:"aud"`
	Iss    string `json:"iss"`
	Exp    int64  `json:"exp"`
	Jti    string `json:"jti"`
}

// mintJWT builds an HS256-signed session token for userID, signed with
// secret. A fresh jti per call — the gateway's JtiTracker rejects a
// replayed jti, so minting once per connection (not once per process) is
// required. Hand-rolled (hmac+sha256+base64) rather than a JWT dependency:
// the claim set is fixed and tiny, and this keeps rsx-term dependency-free
// for its auth path.
func mintJWT(userID uint32, secret string) string {
	now := time.Now()
	claims := jwtClaims{
		Sub:    fmt.Sprintf("rsx-term:%d", userID),
		UserID: userID,
		Aud:    "rsx-gateway",
		Iss:    "rsx-auth",
		Exp:    now.Add(jwtTTL).Unix(),
		Jti:    fmt.Sprintf("rsx-term-%d-%d", userID, now.UnixNano()),
	}
	claimsJSON, err := json.Marshal(claims)
	if err != nil {
		// A struct of concrete scalar/string fields never fails to
		// marshal; this is an invariant, not a runtime condition.
		panic("conn: mintJWT: json.Marshal of scalar claims failed: " + err.Error())
	}

	headerB64 := base64.RawURLEncoding.EncodeToString([]byte(jwtHeader))
	claimsB64 := base64.RawURLEncoding.EncodeToString(claimsJSON)
	signingInput := headerB64 + "." + claimsB64

	mac := hmac.New(sha256.New, []byte(secret))
	// hash.Hash.Write never returns an error (io.Writer contract note in
	// the hash package); no error to check.
	mac.Write([]byte(signingInput))
	sigB64 := base64.RawURLEncoding.EncodeToString(mac.Sum(nil))

	return signingInput + "." + sigB64
}
