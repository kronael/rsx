package assistant

import (
	"os"
	"strings"
	"testing"
	"time"
)

// TestLLMSmoke drives the REAL assistant client against a live arizuko
// /chat/{token} endpoint, one turn per example coin (SOL, ETH, XRP), and
// asserts each returns a non-empty on-topic reply. It is the "the LLM actually
// works" check.
//
// Gated on RSX_TERM_ASSIST (the live chat URL incl. token) — skipped when
// unset, so the normal offline suite and CI never dial. Run it after standing
// up the stack:
//
//	RSX_TERM_ASSIST=$(.ship/45-ARIZUKO-LLM/chat-token.sh | awk -F= '/^RSX_TERM_ASSIST=/{print $2}') \
//	  go test ./assistant/ -run TestLLMSmoke -v -count=1
func TestLLMSmoke(t *testing.T) {
	url := os.Getenv("RSX_TERM_ASSIST")
	if url == "" {
		t.Skip("set RSX_TERM_ASSIST to a live /chat/{token} URL to run the LLM smoke")
	}
	c := New(url)
	coins := []struct {
		sym string
		ctx string
	}{
		{"SOL", "[RSX CONTEXT] symbol=SOL mid=98.40\nbids: 98.38x120, 98.35x80\nasks: 98.42x15, 98.45x60\n[TRADER STATE] flat.\nQ: one line — which side is heavier and the spread in bps?"},
		{"ETH", "[RSX CONTEXT] symbol=ETH mid=3420.0\nbids: 3419.5x40, 3419.0x12\nasks: 3421.0x5, 3423.0x30\n[TRADER STATE] long 10 @ 3400.\nQ: one line — spread in bps and my uPnL sign?"},
		{"XRP", "[RSX CONTEXT] symbol=XRP mid=0.6100\nbids: 0.6098x9000, 0.6095x4000\nasks: 0.6102x1200, 0.6110x8000\n[TRADER STATE] short 5000 @ 0.6200.\nQ: one line — where is the wall?"},
	}
	for _, coin := range coins {
		topic := "smoke-" + coin.sym
		c.Ask(topic, coin.ctx)
		reply := waitReply(t, c, topic, 150*time.Second)
		if strings.TrimSpace(reply) == "" {
			t.Fatalf("%s: no reply within timeout", coin.sym)
		}
		t.Logf("%s → %s", coin.sym, strings.ReplaceAll(reply, "\n", " "))
	}
}

// waitReply drains the client's event stream for a Reply on the given topic,
// failing on a Failed event for that topic, returning "" on timeout.
func waitReply(t *testing.T, c *Client, topic string, d time.Duration) string {
	t.Helper()
	deadline := time.After(d)
	for {
		select {
		case ev := <-c.Events():
			switch e := ev.(type) {
			case Reply:
				if e.Topic == topic {
					return e.Text
				}
			case Failed:
				if e.Topic == topic {
					t.Fatalf("%s failed: %s", topic, e.Err)
				}
			}
		case <-deadline:
			return ""
		}
	}
}
