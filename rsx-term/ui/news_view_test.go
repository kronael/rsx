package ui

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"rsx-term/assistant"
	"rsx-term/conn"
	"rsx-term/news"
	"rsx-term/wire"
)

// fakeNews is a fixed in-memory news source for view tests.
type fakeNews struct {
	markers []news.Marker // newest first
}

func (f *fakeNews) Markers(sinceNs, untilNs int64) []news.Marker {
	var out []news.Marker
	for _, m := range f.markers {
		if m.TsNs >= sinceNs && m.TsNs <= untilNs {
			out = append(out, m)
		}
	}
	return out
}

func (f *fakeNews) All() []news.Marker { return f.markers }
func (f *fakeNews) Enabled() bool      { return true }

// newsModel is a two-symbol stream model with a fake feed, on the news
// screen.
func newsModel(t *testing.T, mock *conn.MockGateway) Model {
	t.Helper()
	m := New(Config{
		Symbol:   "PENGU-PERP",
		SymbolID: 10,
		Sub:      mock,
		PriceDec: 6,
		QtyDec:   4,
		Tick:     1,
		Stream:   true,
		Instruments: []Instrument{
			{ID: 10, Name: "PENGU-PERP", PriceDec: 6, QtyDec: 4, Tick: 1, Sector: "meme"},
			{ID: 3, Name: "SOL-PERP", PriceDec: 4, QtyDec: 6, Tick: 1, Sector: "majors"},
		},
		News: &fakeNews{markers: []news.Marker{
			{TsNs: 2e18, Text: "Exchange halts withdrawals", Source: "Twitter", Tier: 3},
			{TsNs: 1e18, Text: "Binance lists SOL pair", Source: "Blogs", Symbols: []string{"SOLUSDT"}, Tier: 2},
		}},
	})
	m = apply(m, tea.WindowSizeMsg{Width: 110, Height: 32})
	m = apply(m, wire.Snapshot{
		SymbolID: 3,
		Bids:     []wire.Level{{Px: 1_499_950, Qty: 40_000_000, Count: 2}},
		Asks:     []wire.Level{{Px: 1_500_050, Qty: 35_000_000, Count: 1}},
		Seq:      1,
	})
	m = apply(m, binTickMsg(time.Now()))
	m = press(m, "tab") // book → news
	return m
}

func TestNewsViewSectorMapAndFeed(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	plain := stripANSI(m.View())
	for _, want := range []string{"majors", "meme", "SOL", "PENGU", "halts withdrawals", "lists SOL pair", "‼"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("news view missing %q:\n%s", want, plain)
		}
	}
	if got := strings.Count(m.View(), "\n") + 1; got != 32 {
		t.Fatalf("news view = %d lines, want a fixed 32", got)
	}
}

func TestNewsSearchFilters(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "/")
	for _, r := range "sol" {
		m = press(m, string(r))
	}
	plain := stripANSI(m.View())
	if strings.Contains(plain, "halts withdrawals") {
		t.Fatalf("search should filter the feed:\n%s", plain)
	}
	if !strings.Contains(plain, "lists SOL pair") || !strings.Contains(plain, "search: sol_") {
		t.Fatalf("query row missing:\n%s", plain)
	}
	// While typing, letters must NOT jump views or trade.
	if m.screen != screenNews {
		t.Fatalf("typing in search left the news screen")
	}
	m = press(m, "esc")
	if m.newsQuery != "" {
		t.Fatalf("esc should clear the search")
	}
}

func TestNewsSelectionAndHandoff(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "j") // select the second (older, SOL-linked) headline
	m = press(m, "enter")
	if m.screen != screenLLM {
		t.Fatalf("enter should open the assistant, got %v", m.screen)
	}
	if m.assistCtx == nil {
		t.Fatalf("handoff must package a context")
	}
	ctx := *m.assistCtx
	if ctx.Origin != news.OriginNews {
		t.Fatalf("a headline handoff should tag OriginNews, got %v", ctx.Origin)
	}
	if ctx.Symbol != "SOL-PERP" || ctx.Venue != "rsx" {
		t.Fatalf("headline should link to SOL's market: %+v", ctx)
	}
	if ctx.Headline == nil || ctx.Headline.Text != "Binance lists SOL pair" {
		t.Fatalf("wrong headline packaged: %+v", ctx.Headline)
	}
	if len(ctx.Bids) != 1 || ctx.Bids[0].Px != 1_499_950 {
		t.Fatalf("book snapshot not frozen into the context: %+v", ctx.Bids)
	}
	if ctx.MidPx != 1_500_000 {
		t.Fatalf("mid at handoff = %d", ctx.MidPx)
	}
	// The snapshot is a copy: the live book folding on must not mutate it.
	m = apply(m, wire.Delta{SymbolID: 3, Side: 0, Px: 1_499_950, Qty: 0, Seq: 2})
	if len(m.assistCtx.Bids) != 1 || m.assistCtx.Bids[0].Qty != 40_000_000 {
		t.Fatalf("handoff context must stay frozen: %+v", m.assistCtx.Bids)
	}
}

func TestLLMViewRendersHandoffAndPlaceholder(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "j")
	m = press(m, "enter")
	plain := stripANSI(m.View())
	for _, want := range []string{"ASSISTANT", "rsx · SOL-PERP", "lists SOL pair", "150.0000", "placeholder"} {
		if !strings.Contains(plain, want) {
			t.Fatalf("assistant view missing %q:\n%s", want, plain)
		}
	}
	m = press(m, "esc")
	if m.screen != screenNews {
		t.Fatalf("esc should return to the news view")
	}
}

func TestLLMViewWithoutContext(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "tab") // news → llm without a handoff
	plain := stripANSI(m.View())
	if !strings.Contains(plain, "no context yet") {
		t.Fatalf("empty assistant should say so:\n%s", plain)
	}
}

func TestNewsLetterJumpsToBook(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	code := m.instrumentFor("rsx", 3).Code
	m = press(m, code)
	if m.screen != screenBook || m.active != 3 {
		t.Fatalf("letter should jump into the symbol's book: screen %v active %d", m.screen, m.active)
	}
}

func TestBookNKeyOpensNews(t *testing.T) {
	m := streamModel(t, &conn.MockGateway{})
	m = press(m, "n")
	if m.screen != screenNews {
		t.Fatalf("n should open the news view from the book")
	}
}

// TestAssistReplyLinesOfflineExact byte-locks the OFFLINE reply pane: with no
// client wired, assistReplyLines must be the exact pre-wiring placeholder (ANSI
// included), regardless of budget — the offline path stays byte-identical.
func TestAssistReplyLinesOfflineExact(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{}) // Config.Assist unset → offline
	want := []string{
		"",
		StyleMuted.Render("  ── assistant reply ────────────────────────────"),
		StyleDerived.Render("  ~ placeholder — wiring an LLM is a follow-up; nothing here is generated"),
	}
	got := m.assistReplyLines(20)
	if len(got) != len(want) {
		t.Fatalf("offline reply section = %d lines, want %d: %#v", len(got), len(want), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("offline reply line %d drifted:\n got %q\nwant %q", i, got[i], want[i])
		}
	}
}

// TestLLMOfflinePaneUnchanged proves the whole assistant screen is unchanged
// offline: the placeholder renders, the header still says "no model wired", and
// no live chrome leaks in.
func TestLLMOfflinePaneUnchanged(t *testing.T) {
	m := newsModel(t, &conn.MockGateway{})
	m = press(m, "j")
	m = press(m, "enter")
	plain := stripANSI(m.View())
	if !strings.Contains(plain, "~ placeholder — wiring an LLM is a follow-up; nothing here is generated") {
		t.Fatalf("offline pane must be the exact placeholder:\n%s", plain)
	}
	if !strings.Contains(plain, "no model wired") {
		t.Fatalf("offline header must be unchanged:\n%s", plain)
	}
	for _, forbid := range []string{"live — arizuko", "  > ", "waiting for the assistant"} {
		if strings.Contains(plain, forbid) {
			t.Fatalf("offline pane leaked live chrome %q:\n%s", forbid, plain)
		}
	}
	if m.assistBusy {
		t.Fatalf("offline handoff must never set the busy flag")
	}
}

// pumpAssist forwards the client's streamed events into the model, standing in
// for main.go's drainEvents, until a reply/failure clears the busy flag.
func pumpAssist(t *testing.T, m Model) Model {
	t.Helper()
	deadline := time.After(3 * time.Second)
	for m.assistBusy {
		select {
		case ev := <-m.assist.Events():
			m = apply(m, ev)
		case <-deadline:
			t.Fatal("timed out waiting for the assistant reply")
		}
	}
	return m
}

// TestLLMLiveHandoffStreamsReply drives the live path end to end over a local
// httptest SSE server (no live arizuko): a headline handoff posts the context
// turn, opens a busy thread, and the streamed reply renders in the transcript.
func TestLLMLiveHandoffStreamsReply(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		io.WriteString(w, "event: message\ndata: {\"role\":\"assistant\",\"content\":\"SOL bid-heavy at 150.0\"}\n\n")
		io.WriteString(w, "event: round_done\ndata: {\"status\":\"done\"}\n\n")
	}))
	defer srv.Close()

	m := newsModel(t, &conn.MockGateway{})
	m.assist = assistant.New(srv.URL)
	m = press(m, "j")
	m = press(m, "enter") // handoff → posts the context turn
	if !m.assistBusy || m.assistTopic == "" {
		t.Fatalf("a live handoff should open a busy thread: busy=%v topic=%q", m.assistBusy, m.assistTopic)
	}
	if !strings.HasPrefix(m.assistTopic, "t-") {
		t.Fatalf("topic scheme is t-<ms>-<venue>-<symbol>: %q", m.assistTopic)
	}
	m = pumpAssist(t, m)
	plain := stripANSI(m.View())
	if !strings.Contains(plain, "SOL bid-heavy at 150.0") {
		t.Fatalf("the streamed reply should render in the transcript:\n%s", plain)
	}
	if !strings.Contains(plain, "live — arizuko") {
		t.Fatalf("the live header should replace the placeholder:\n%s", plain)
	}
}

// TestLLMLiveTypedFollowUp checks the typing grammar on the live screen: a
// printable key extends the draft (not a view jump), enter posts it on the SAME
// thread and echoes a "you" row.
func TestLLMLiveTypedFollowUp(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		io.WriteString(w, "event: message\ndata: {\"role\":\"assistant\",\"content\":\"net +5 lots\"}\n\n")
		io.WriteString(w, "event: round_done\ndata: {\"status\":\"done\"}\n\n")
	}))
	defer srv.Close()

	m := newsModel(t, &conn.MockGateway{})
	m.assist = assistant.New(srv.URL)
	m = press(m, "j")
	m = press(m, "enter") // handoff
	m = pumpAssist(t, m)  // drain the opening reply
	first := m.assistTopic

	for _, r := range "hi" {
		m = press(m, string(r))
	}
	if m.assistInput != "hi" || m.screen != screenLLM {
		t.Fatalf("typing must land in the draft and stay on the LLM screen: %q screen %v", m.assistInput, m.screen)
	}
	m = press(m, "enter")
	if m.assistTopic != first {
		t.Fatalf("a follow-up must reuse the thread: %q → %q", first, m.assistTopic)
	}
	if !m.assistBusy {
		t.Fatalf("sending a follow-up should set busy")
	}
	if !strings.Contains(stripANSI(m.View()), "hi") {
		t.Fatalf("the typed message should echo in the transcript:\n%s", stripANSI(m.View()))
	}
	m = pumpAssist(t, m)
	if !strings.Contains(stripANSI(m.View()), "net +5 lots") {
		t.Fatalf("the follow-up reply should render:\n%s", stripANSI(m.View()))
	}
}

// liveThreadModel is an enabled model parked on a thread WITHOUT dialing (no
// Ask), for pure event-folding assertions.
func liveThreadModel(t *testing.T, topic string) Model {
	t.Helper()
	m := newsModel(t, &conn.MockGateway{})
	m.assist = assistant.New("http://assist.invalid/chat/x") // enabled; never Asked → no dial
	m.assistTopic = topic
	ctx := news.PackageNews("rsx", "SOL-PERP", 1, news.Marker{Text: "x", Source: "s"}, nil, nil, 1_500_000)
	m.assistCtx = &ctx
	m.assistIns = m.instrumentFor("rsx", 3)
	m.screen = screenLLM
	return m
}

// TestLLMIgnoresStaleThreadReply proves thread isolation: a reply for a prior
// thread is dropped, the current thread's reply renders.
func TestLLMIgnoresStaleThreadReply(t *testing.T) {
	m := liveThreadModel(t, "t-current")
	m = apply(m, assistant.Reply{Topic: "t-old", Text: "ghost"})
	if strings.Contains(stripANSI(m.View()), "ghost") {
		t.Fatalf("a reply for a stale thread must not render")
	}
	m = apply(m, assistant.Reply{Topic: "t-current", Text: "real-answer"})
	if !strings.Contains(stripANSI(m.View()), "real-answer") {
		t.Fatalf("the current thread's reply should render")
	}
}

// TestLLMFailureRendersUnreachable proves a failure surfaces honestly and never
// as fabricated content.
func TestLLMFailureRendersUnreachable(t *testing.T) {
	m := liveThreadModel(t, "t-1")
	m.assistBusy = true
	m = apply(m, assistant.Failed{Topic: "t-1", Err: "timed out"})
	if m.assistBusy {
		t.Fatalf("a failure should clear the busy marker")
	}
	if !strings.Contains(stripANSI(m.View()), "assistant unreachable — timed out") {
		t.Fatalf("failure must render honestly:\n%s", stripANSI(m.View()))
	}
}
