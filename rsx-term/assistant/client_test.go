package assistant

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

// collect drains up to want events (or fails after a timeout), so a test never
// hangs on a missing frame.
func collect(t *testing.T, c *Client, want int) []any {
	t.Helper()
	var got []any
	deadline := time.After(3 * time.Second)
	for len(got) < want {
		select {
		case ev := <-c.Events():
			got = append(got, ev)
		case <-deadline:
			t.Fatalf("timed out after %d/%d events: %+v", len(got), want, got)
		}
	}
	return got
}

func TestClientStreamsAssistantReply(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Header.Get("Accept") != "text/event-stream" {
			t.Errorf("missing SSE Accept header: %q", r.Header.Get("Accept"))
		}
		var body chatRequest
		if err := json.NewDecoder(r.Body).Decode(&body); err != nil {
			t.Errorf("bad request body: %v", err)
		}
		if body.Topic != "t-1" || body.Content != "ping" {
			t.Errorf("request body = %+v", body)
		}
		w.Header().Set("Content-Type", "text/event-stream")
		io.WriteString(w, ": ok\n\n")
		io.WriteString(w, "event: status\ndata: {\"role\":\"assistant\",\"content\":\"⏳ thinking\"}\n\n")
		io.WriteString(w, "event: message\ndata: {\"role\":\"user\",\"content\":\"ping\"}\n\n")
		io.WriteString(w, "event: message\ndata: {\"role\":\"assistant\",\"content\":\"pong 42\"}\n\n")
		io.WriteString(w, "event: round_done\ndata: {\"turn_id\":\"x\",\"status\":\"done\"}\n\n")
	}))
	defer srv.Close()

	c := New(srv.URL)
	c.Ask("t-1", "ping")

	got := collect(t, c, 2)
	status, ok := got[0].(Status)
	if !ok || status.Text != "thinking" || status.Topic != "t-1" {
		t.Fatalf("first event should be the status frame (⏳ stripped): %+v", got[0])
	}
	reply, ok := got[1].(Reply)
	if !ok || reply.Text != "pong 42" || reply.Topic != "t-1" {
		t.Fatalf("second event should be the assistant reply: %+v", got[1])
	}
	// The echoed user frame is NOT surfaced as content.
	for _, ev := range got {
		if r, ok := ev.(Reply); ok && r.Text == "ping" {
			t.Fatalf("the echoed user message must not render as a reply")
		}
	}
}

func TestClientReportsUnreachable(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusBadGateway)
	}))
	defer srv.Close()

	c := New(srv.URL)
	c.Ask("t-9", "x")
	got := collect(t, c, 1)
	fail, ok := got[0].(Failed)
	if !ok || fail.Topic != "t-9" || fail.Err != fmt.Sprintf("HTTP %d", http.StatusBadGateway) {
		t.Fatalf("a non-200 must surface as an honest Failed: %+v", got[0])
	}
}

func TestClientReportsDialFailure(t *testing.T) {
	// A URL nothing is listening on: the dial fails, surfaced as Failed — never
	// a fabricated reply.
	c := New("http://127.0.0.1:1/chat/nope")
	c.Ask("t-2", "x")
	got := collect(t, c, 1)
	if _, ok := got[0].(Failed); !ok {
		t.Fatalf("an unreachable endpoint must surface as Failed: %+v", got[0])
	}
}

func TestDisabledClientMakesNoCall(t *testing.T) {
	var nilClient *Client
	if nilClient.Enabled() {
		t.Fatal("a nil client must be disabled")
	}
	nilClient.Ask("t", "x")        // must not panic or dial
	if nilClient.Events() != nil { // nil stream — nothing to drain
		t.Fatal("a nil client must have no event stream")
	}
	empty := New("")
	if empty.Enabled() {
		t.Fatal("an empty URL must be disabled")
	}
	empty.Ask("t", "x") // no-op, no goroutine, no dial
}
