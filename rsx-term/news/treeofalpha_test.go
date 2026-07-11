package news

import (
	"strings"
	"testing"
	"time"
)

func TestDecodeHeadline(t *testing.T) {
	data := []byte(`{"title":"Binance lists WIF","source":"Twitter","symbols":["WIFUSDT"],"time":1700000000000}`)
	m, ok := DecodeHeadline(data)
	if !ok {
		t.Fatalf("headline should decode")
	}
	if m.Text != "Binance lists WIF" || m.Source != "Twitter" {
		t.Fatalf("fields: %+v", m)
	}
	if m.TsNs != 1700000000000*int64(time.Millisecond) {
		t.Fatalf("ts: %d", m.TsNs)
	}
	if len(m.Symbols) != 1 || m.Symbols[0] != "WIFUSDT" {
		t.Fatalf("symbols: %v", m.Symbols)
	}
	if m.Tier != 2 { // "lists" is a high-severity word
		t.Fatalf("tier = %d, want 2", m.Tier)
	}
}

func TestDecodeHeadlineRejectsNonHeadlines(t *testing.T) {
	for _, frame := range []string{
		`{"ping":1}`,
		`{"title":"","time":1700000000000}`,
		`{"title":"no timestamp"}`,
		`not json`,
	} {
		if _, ok := DecodeHeadline([]byte(frame)); ok {
			t.Fatalf("frame %q must not decode as a headline", frame)
		}
	}
}

func TestDeriveSeverityGrades(t *testing.T) {
	cases := map[string]int{
		"Protocol X exploited for $40M": 3,
		"Exchange halts withdrawals":    3,
		"SEC approves spot ETF":         2,
		"Fed holds rates steady":        2,
		"Some coin does something":      0,
	}
	for text, want := range cases {
		if got := DeriveSeverity(text, nil); got != want {
			t.Fatalf("severity(%q) = %d, want %d", text, got, want)
		}
	}
	if DeriveSeverity("routine tagged item", []string{"BTC"}) != 1 {
		t.Fatalf("symbol-tagged routine news should read tier 1")
	}
}

func TestBufferedMarkersWindowAndOrder(t *testing.T) {
	src := NewTreeOfAlpha()
	// Not enabled: buffered markers stay invisible on the rail.
	src.push(Marker{TsNs: 100, Text: "early"})
	if src.Markers(0, 200) != nil {
		t.Fatalf("disabled source must return no markers")
	}
	src.mu.Lock()
	src.enabled = true
	src.mu.Unlock()
	src.push(Marker{TsNs: 300, Text: "mid"})
	src.push(Marker{TsNs: 500, Text: "late"})
	got := src.Markers(200, 400)
	if len(got) != 1 || got[0].Text != "mid" {
		t.Fatalf("window query: %+v", got)
	}
	all := src.All()
	if len(all) != 3 || all[0].Text != "late" {
		t.Fatalf("All should list newest first: %+v", all)
	}
}

func TestRingCaps(t *testing.T) {
	src := NewTreeOfAlpha()
	for i := 0; i < bufferCap+10; i++ {
		src.push(Marker{TsNs: int64(i), Text: "x"})
	}
	if len(src.markers) != bufferCap {
		t.Fatalf("ring len %d, want %d", len(src.markers), bufferCap)
	}
	if src.markers[0].TsNs != 10 {
		t.Fatalf("oldest retained should be 10, got %d", src.markers[0].TsNs)
	}
}

func TestOffSourceStaysOff(t *testing.T) {
	var src Source = Off{}
	if src.Enabled() || src.Markers(0, 1<<62) != nil {
		t.Fatalf("Off must never report markers")
	}
}

func TestDialSiteIsOnlyInStart(t *testing.T) {
	// Guard the no-network-by-default contract at the API level: the
	// constructor performs no I/O and the type starts disabled.
	src := NewTreeOfAlpha()
	if src.Enabled() {
		t.Fatalf("a fresh TreeOfAlpha must be disabled until Start")
	}
	if !strings.HasPrefix(TreeOfAlphaURL, "wss://news.treeofalpha.com") {
		t.Fatalf("endpoint drifted: %s", TreeOfAlphaURL)
	}
}
