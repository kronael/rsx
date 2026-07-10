package conn

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// symbolsBody mirrors the gateway's /v1/symbols shape (rsx-gateway rest.rs).
const symbolsBody = `{"symbols":[` +
	`{"id":10,"tick_size":1,"lot_size":100000,"price_decimals":6,"qty_decimals":4},` +
	`{"id":11,"tick_size":5,"lot_size":1000,"price_decimals":2,"qty_decimals":0}]}`

func symbolsServer(t *testing.T, status int, body string) *httptest.Server {
	t.Helper()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/v1/symbols" {
			http.NotFound(w, r)
			return
		}
		w.WriteHeader(status)
		_, _ = w.Write([]byte(body)) // test server; nothing to do on disconnect
	}))
	t.Cleanup(server.Close)
	return server
}

func TestFetchSymbolConfig(t *testing.T) {
	server := symbolsServer(t, http.StatusOK, symbolsBody)
	// The terminal holds a ws:// URL; the fetch converts scheme, same host:port.
	wsURL := "ws://" + strings.TrimPrefix(server.URL, "http://")

	cfg, err := FetchSymbolConfig(wsURL, 10)
	if err != nil {
		t.Fatalf("fetch: %v", err)
	}
	want := SymbolConfig{ID: 10, TickSize: 1, LotSize: 100000, PriceDec: 6, QtyDec: 4}
	if cfg != want {
		t.Fatalf("cfg = %+v, want %+v", cfg, want)
	}
}

func TestFetchSymbolConfigUnknownSymbol(t *testing.T) {
	server := symbolsServer(t, http.StatusOK, symbolsBody)
	if _, err := FetchSymbolConfig(server.URL, 99); err == nil {
		t.Fatal("unknown symbol should error, not silently default")
	}
}

func TestFetchSymbolConfigServerError(t *testing.T) {
	server := symbolsServer(t, http.StatusInternalServerError, "")
	if _, err := FetchSymbolConfig(server.URL, 10); err == nil {
		t.Fatal("non-200 should error")
	}
}

func TestFetchSymbolConfigDeadEndpoint(t *testing.T) {
	// A port nothing listens on: the fetch must fail fast (bounded by its
	// timeout), not hang the terminal's startup.
	if _, err := FetchSymbolConfig("ws://127.0.0.1:1", 10); err == nil {
		t.Fatal("dead endpoint should error")
	}
}

func TestRestURL(t *testing.T) {
	cases := [][2]string{
		{"ws://127.0.0.1:8080", "http://127.0.0.1:8080"},
		{"wss://gw.example.com", "https://gw.example.com"},
		{"http://already.http", "http://already.http"},
	}
	for _, c := range cases {
		if got := restURL(c[0]); got != c[1] {
			t.Fatalf("restURL(%q) = %q, want %q", c[0], got, c[1])
		}
	}
}
