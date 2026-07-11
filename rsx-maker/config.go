package main

import (
	"os"
	"strconv"
	"strings"
	"time"
)

// Config holds every runtime knob, all sourced from the environment
// to mirror the Python maker's env surface (do_maker_start in
// rsx-playground/server.py sets these when it spawns the process).
type Config struct {
	GatewayURL   string
	MarketdataWS string
	SymbolsURL   string
	ConfigFile   string
	JWTSecret    string
	UserID       uint32
	Symbols      []uint32
	SpreadBps    int64
	QtyPerLevel  int64
	Levels       int
	Refresh      time.Duration
	MidOverride  int64
	HasMid       bool
}

// loadConfig reads the environment into a Config, applying the same
// defaults as market_maker.py's __main__ block.
func loadConfig() Config {
	c := Config{
		GatewayURL:   envStr("GATEWAY_URL", "ws://127.0.0.1:8080"),
		MarketdataWS: envStr("MARKETDATA_WS", "ws://127.0.0.1:8180"),
		SymbolsURL:   envStr("RSX_SYMBOLS_URL", ""),
		ConfigFile:   envStr("RSX_MAKER_CONFIG_FILE", ""),
		JWTSecret:    envStr("RSX_GW_JWT_SECRET", ""),
		UserID:       uint32(envInt("RSX_MAKER_USER", 99)),
		Symbols:      []uint32{uint32(envInt("RSX_MAKER_SYMBOL", 10))},
		SpreadBps:    envInt("RSX_MAKER_SPREAD_BPS", 10),
		QtyPerLevel:  envInt("RSX_MAKER_QTY", 10),
		Levels:       int(envInt("RSX_MAKER_LEVELS", 5)),
		Refresh:      time.Duration(envInt("RSX_MAKER_REFRESH_MS", 2000)) * time.Millisecond,
	}
	if v := strings.TrimSpace(os.Getenv("RSX_MAKER_MID_OVERRIDE")); v != "" {
		if n, err := strconv.ParseInt(v, 10, 64); err == nil {
			c.MidOverride = n
			c.HasMid = true
		}
	}
	if c.SymbolsURL == "" {
		c.SymbolsURL = deriveSymbolsURL(c.GatewayURL)
	}
	return c
}

// deriveSymbolsURL turns a gateway ws:// URL into the http:// /v1/symbols
// endpoint, matching the Python maker's fallback.
func deriveSymbolsURL(gatewayURL string) string {
	u := gatewayURL
	u = strings.Replace(u, "wss://", "https://", 1)
	u = strings.Replace(u, "ws://", "http://", 1)
	u = strings.TrimRight(u, "/")
	return u + "/v1/symbols"
}

func envStr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

func envInt(key string, def int64) int64 {
	v := os.Getenv(key)
	if v == "" {
		return def
	}
	n, err := strconv.ParseInt(strings.TrimSpace(v), 10, 64)
	if err != nil {
		return def
	}
	return n
}
