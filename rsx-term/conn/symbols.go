package conn

// Symbol-config discovery: the gateway serves GET /v1/symbols (same port as
// the WS — non-upgrade requests fall through to REST, rsx-gateway handler.rs)
// with each symbol's tick/lot/decimals. Fetching it at startup means the
// terminal displays the right precision without per-symbol env plumbing; env
// vars stay as an explicit override, and a fetch failure falls back to the
// caller's defaults — config discovery must never keep the terminal from
// starting.

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"
)

// SymbolConfig is one entry of the gateway's /v1/symbols response.
type SymbolConfig struct {
	ID       uint32 `json:"id"`
	TickSize int64  `json:"tick_size"`
	LotSize  int64  `json:"lot_size"`
	PriceDec int    `json:"price_decimals"`
	QtyDec   int    `json:"qty_decimals"`
}

// symbolsResponse is the /v1/symbols envelope.
type symbolsResponse struct {
	Symbols []SymbolConfig `json:"symbols"`
}

// fetchTimeout bounds the startup config fetch: long enough for a local
// gateway, short enough that a dead endpoint doesn't stall launch.
const fetchTimeout = 2 * time.Second

// FetchSymbolConfig GETs the gateway's /v1/symbols and returns the config for
// symbolID. gwURL is the WS URL the terminal already has (ws://host:port);
// the REST endpoint lives on the same listener, so only the scheme changes.
func FetchSymbolConfig(gwURL string, symbolID uint32) (SymbolConfig, error) {
	url := restURL(gwURL) + "/v1/symbols"
	client := &http.Client{Timeout: fetchTimeout}
	resp, err := client.Get(url)
	if err != nil {
		return SymbolConfig{}, fmt.Errorf("fetch %s: %w", url, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return SymbolConfig{}, fmt.Errorf("fetch %s: status %d", url, resp.StatusCode)
	}
	var body symbolsResponse
	if err := json.NewDecoder(resp.Body).Decode(&body); err != nil {
		return SymbolConfig{}, fmt.Errorf("decode %s: %w", url, err)
	}
	for _, s := range body.Symbols {
		if s.ID == symbolID {
			return s, nil
		}
	}
	return SymbolConfig{}, fmt.Errorf("symbol %d not in %s (%d symbols)", symbolID, url, len(body.Symbols))
}

// restURL converts the gateway WS URL to its HTTP sibling on the same
// host:port (ws→http, wss→https). A URL with no ws scheme is returned as-is,
// assuming it is already an http(s) URL.
func restURL(gwURL string) string {
	if rest, ok := strings.CutPrefix(gwURL, "wss://"); ok {
		return "https://" + rest
	}
	if rest, ok := strings.CutPrefix(gwURL, "ws://"); ok {
		return "http://" + rest
	}
	return gwURL
}
