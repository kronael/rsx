package assistant

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"
)

// idleCutoff bounds a turn by inactivity, not total time: each received frame
// resets it, so a cold container that takes tens of seconds before its first
// token is fine, but a wedged turn is abandoned. arizuko runs Claude Code in a
// fresh Docker container per turn (seconds, cold start possibly tens of
// seconds), so this is generous by design.
const idleCutoff = 180 * time.Second

// eventBuffer bounds the pending-event channel; the terminal drains it every
// tick, so a small buffer is plenty of slack for a bursty stream.
const eventBuffer = 64

// statusGlyph marks arizuko's progress frames — a "⏳ " content prefix (or an
// event: status frame). Stripped before the status text surfaces.
const statusGlyph = "⏳"

// httpClient has no client-wide timeout on purpose: an SSE turn streams for as
// long as the agent is producing, and the idle context cutoff — not a total
// deadline — is what abandons a wedged turn.
var httpClient = &http.Client{}

// Reply is a complete assistant message for a thread.
type Reply struct {
	Topic string
	Text  string
}

// Status is an agent progress marker (the "⏳" frames) for a thread — a busy
// cue for the status line, never rendered as reply content.
type Status struct {
	Topic string
	Text  string
}

// Failed is a dial / HTTP / timeout failure for a thread — an honest error the
// pane surfaces, never a fabricated reply.
type Failed struct {
	Topic string
	Err   string
}

// Client posts chat turns to a locally deployed arizuko /chat/{token} endpoint
// and streams the SSE reply back as typed events. It is OFF unless a URL is
// configured (RSX_TERM_ASSIST): a nil *Client, or one built from an empty URL,
// makes zero network calls, so the terminal stays offline by default. Mirrors
// news.TreeOfAlpha: the constructor does no I/O, each turn runs in one named
// goroutine (streamTurn), and failures surface as Failed events, never
// fabricated content.
type Client struct {
	url    string
	events chan any
}

// New builds a client for the full chat URL (including the minted route token).
// It does no I/O — the first dial is deferred to Ask's goroutine.
func New(url string) *Client {
	return &Client{url: url, events: make(chan any, eventBuffer)}
}

// Enabled reports whether the client will dial (a URL is configured). Safe on a
// nil receiver — the offline default.
func (c *Client) Enabled() bool { return c != nil && c.url != "" }

// Events is the stream of Reply / Status / Failed messages. A nil client has no
// stream (a nil channel the caller simply never ranges into a dial).
func (c *Client) Events() <-chan any {
	if c == nil {
		return nil
	}
	return c.events
}

// Ask queues one turn on the given thread (topic) with the rendered prompt. It
// is non-blocking and a no-op when disabled — the ONLY dial site is the named
// streamTurn goroutine it launches, gated on Enabled.
func (c *Client) Ask(topic, content string) {
	if !c.Enabled() {
		return
	}
	go c.streamTurn(topic, content)
}

// streamTurn POSTs one turn and streams its SSE reply into the event channel.
// The idle timer cancels the request's context after idleCutoff of silence
// (resetting on every frame), so both a stalled connect and a wedged mid-stream
// abandon cleanly as a timeout rather than hanging the goroutine.
func (c *Client) streamTurn(topic, content string) {
	payload, err := json.Marshal(chatRequest{Content: content, Topic: topic})
	if err != nil {
		c.emit(Failed{Topic: topic, Err: err.Error()})
		return
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	idle := time.AfterFunc(idleCutoff, cancel)
	defer idle.Stop()

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.url, bytes.NewReader(payload))
	if err != nil {
		c.emit(Failed{Topic: topic, Err: err.Error()})
		return
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "text/event-stream")

	resp, err := httpClient.Do(req)
	if err != nil {
		c.emit(Failed{Topic: topic, Err: reason(ctx, err)})
		return
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		c.emit(Failed{Topic: topic, Err: fmt.Sprintf("HTTP %d", resp.StatusCode)})
		return
	}

	scanner := bufio.NewScanner(resp.Body)
	scanner.Buffer(make([]byte, 0, 64*1024), 1<<20)
	var event, data string
	gotReply := false
	for scanner.Scan() {
		idle.Reset(idleCutoff)
		line := scanner.Text()
		switch {
		case line == "":
			done, msg := frame(topic, event, data)
			if msg != nil {
				c.emit(msg)
				if _, ok := msg.(Reply); ok {
					gotReply = true
				}
			}
			if done {
				if !gotReply {
					c.emit(Failed{Topic: topic, Err: "assistant finished without a reply"})
				}
				return
			}
			event, data = "", ""
		case strings.HasPrefix(line, ":"):
			// SSE comment (": ok" keep-alive) — ignore.
		case strings.HasPrefix(line, "event:"):
			event = strings.TrimSpace(line[len("event:"):])
		case strings.HasPrefix(line, "data:"):
			data = strings.TrimSpace(line[len("data:"):])
		}
	}
	if err := scanner.Err(); err != nil {
		c.emit(Failed{Topic: topic, Err: reason(ctx, err)})
	} else if !gotReply {
		c.emit(Failed{Topic: topic, Err: "assistant closed the stream without a reply"})
	}
}

// emit delivers one event to the drain loop.
func (c *Client) emit(msg any) { c.events <- msg }

// chatRequest is the POST body arizuko's /chat endpoint reads (the client-held
// topic keeps one coherent multi-turn thread; arizuko never returns its own
// auto-topic, so we always send ours).
type chatRequest struct {
	Content string `json:"content"`
	Topic   string `json:"topic"`
}

// messageFrame is the SSE data payload for a hub message / status frame (the
// fields we read from webd's {id,role,content,topic,...} JSON).
type messageFrame struct {
	Role    string `json:"role"`
	Content string `json:"content"`
}

// frame turns one complete SSE frame into a typed event. done is true for the
// terminal round_done frame (the caller stops reading); msg is nil for frames
// that carry nothing to render — keep-alives, the echoed user message, or
// empty content. Assistant frames with a "⏳" prefix are progress, not content.
func frame(topic, event, data string) (done bool, msg any) {
	if event == "round_done" {
		return true, nil
	}
	if data == "" {
		return false, nil
	}
	var f messageFrame
	if json.Unmarshal([]byte(data), &f) != nil {
		return false, nil
	}
	text := strings.TrimSpace(f.Content)
	if text == "" {
		return false, nil
	}
	if event == "status" || strings.HasPrefix(text, statusGlyph) {
		return false, Status{Topic: topic, Text: strings.TrimSpace(strings.TrimPrefix(text, statusGlyph))}
	}
	if f.Role == "assistant" {
		return false, Reply{Topic: topic, Text: text}
	}
	return false, nil // role "user" is our own echoed prompt — ignore.
}

// reason labels a request error: a cutoff-cancelled context reads as a timeout,
// anything else surfaces the underlying error.
func reason(ctx context.Context, err error) string {
	if ctx.Err() != nil {
		return "timed out"
	}
	return err.Error()
}
