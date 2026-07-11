package ui

import (
	"fmt"
	"sort"
	"strings"
)

// The streaming terminal's keymap is ONE data table (the k9s pattern): the
// dispatcher, the per-screen persistent hint line, and the ? help overlay
// all render from it, so they can never drift apart. Verb keys are
// REBINDABLE (RSX_TERM_KEYMAP, a JSON {"action":"key"} file); key CLASSES
// (digits as presets/counts, shifted digits as crosses, letters as symbol
// selectors) are structural grammar and stay fixed. The x / F9 / "/"
// prefixes are the terminal's leader-style chords: each opens a one-shot
// capture mode (symbol code, venue letter, search text) instead of a global
// leader namespace — flat per-view maps with modal prefixes is the
// convention traders already know (vim/k9s/lazygit).

// action is a rebindable command id (the JSON key in RSX_TERM_KEYMAP).
type action string

const (
	actNone       action = ""
	actQuit       action = "quit"
	actHelp       action = "help"
	actNextView   action = "next-view"
	actPrevView   action = "prev-view"
	actVenuePick  action = "venue-pick"
	actReduceOnly action = "reduce-only"
	actPostOnly   action = "post-only"

	actSwitchSymbol action = "switch-symbol"
	actOpenNews     action = "open-news"
	actBuySide      action = "buy-side"
	actSellSide     action = "sell-side"
	actCursorDown   action = "cursor-down"
	actCursorUp     action = "cursor-up"
	actCursorBid    action = "cursor-bid"
	actCursorAsk    action = "cursor-ask"
	actRowUp        action = "row-up"   // microscope: cursor toward older rows
	actRowDown      action = "row-down" // microscope: cursor toward newer rows
	actFreeze       action = "freeze"   // microscope: hand the cursor row to the assistant
	actPlace        action = "place"
	actCancel       action = "cancel"
	actQuitBook     action = "quit-book" // esc in the book = quit (top layer)

	actSearch   action = "search"
	actFeedDown action = "feed-down"
	actFeedUp   action = "feed-up"
	actHandoff  action = "handoff"
	actBack     action = "back"
)

// binding is one table row: the rebindable primary key, fixed alternates
// (arrows etc.), the help text, and whether it earns a slot on the
// persistent hint line.
type binding struct {
	action action
	key    string
	alts   []string
	help   string
	hint   string // short hint-line label ("" = not on the hint line)
	danger bool   // capital-at-risk: rendered in the ask hue in help
}

// classDoc documents a fixed key CLASS for the help overlay (not
// dispatched via the table — the grammar in the handlers).
type classDoc struct {
	keys   string
	help   string
	danger bool
}

// keymap is the full table, per screen plus the globals.
type keymap struct {
	global []binding
	book   []binding
	news   []binding
	llm    []binding

	bookClasses []classDoc
	newsClasses []classDoc
	llmClasses  []classDoc
}

// defaultKeymap is the shipped table.
func defaultKeymap() *keymap {
	return &keymap{
		global: []binding{
			{action: actQuit, key: "q", alts: []string{"ctrl+c"}, help: "quit", hint: "q quit"},
			{action: actNextView, key: "tab", help: "next view (book → news → assistant)", hint: "tab view"},
			{action: actPrevView, key: "shift+tab", help: "previous view"},
			{action: actVenuePick, key: "f9", help: "venue picker (then the venue's letter)"},
			{action: actReduceOnly, key: "r", help: "toggle reduce-only (applies to every order)", hint: "r RO"},
			{action: actPostOnly, key: "p", help: "toggle post-only (applies to resting orders)"},
			{action: actHelp, key: "?", help: "this help", hint: "? help"},
		},
		book: []binding{
			{action: actSwitchSymbol, key: "x", help: "symbol switcher (then the symbol's code)", hint: "x symbol"},
			{action: actOpenNews, key: "n", help: "news view", hint: "n news"},
			{action: actBuySide, key: "b", help: "side: buy", hint: "b/s side"},
			{action: actSellSide, key: "s", help: "side: sell"},
			{action: actCursorDown, key: "h", alts: []string{"left"}, help: "cursor one tick down", hint: "h/l cursor"},
			{action: actCursorUp, key: "l", alts: []string{"right"}, help: "cursor one tick up"},
			{action: actCursorBid, key: "j", help: "cursor to the best bid", hint: "j/k touch"},
			{action: actCursorAsk, key: "k", help: "cursor to the best ask"},
			{action: actRowUp, key: "up", help: "microscope: cursor to an older row (freeze, not replay)"},
			{action: actRowDown, key: "down", help: "microscope: cursor to a newer row"},
			{action: actFreeze, key: "enter", help: "freeze the cursor row → assistant", danger: false},
			{action: actPlace, key: "f", help: "place resting limit at the cursor", hint: "f place", danger: true},
			{action: actCancel, key: "d", help: "cancel own order nearest the cursor", hint: "d cancel", danger: true},
			{action: actQuitBook, key: "esc", help: "quit (esc again if the microscope is on turns it off first)"},
		},
		news: []binding{
			{action: actSearch, key: "/", help: "search the feed (type, enter keeps, esc clears)", hint: "/ search"},
			{action: actFeedDown, key: "j", alts: []string{"down"}, help: "select next headline", hint: "j/k select"},
			{action: actFeedUp, key: "k", alts: []string{"up"}, help: "select previous headline"},
			{action: actHandoff, key: "enter", help: "hand the headline + frozen book to the assistant", hint: "enter → assistant"},
			{action: actBack, key: "esc", help: "clear search / back to the book", hint: "esc back"},
		},
		llm: []binding{
			{action: actBack, key: "esc", help: "back to the news view", hint: "esc → news"},
		},
		bookClasses: []classDoc{
			{keys: "1-5", help: "arm a size preset"},
			{keys: "⇧1-5", help: "cross NOW — IOC at the far touch, preset size", danger: true},
		},
		newsClasses: []classDoc{
			{keys: "a-z", help: "jump into the symbol's book view"},
		},
		llmClasses: []classDoc{
			{keys: "type", help: "type to chat · enter sends · esc backs out (when the assistant is wired via RSX_TERM_ASSIST)"},
		},
	}
}

// screenBindings returns a screen's own table.
func (k *keymap) screenBindings(s screen) []binding {
	switch s {
	case screenNews:
		return k.news
	case screenLLM:
		return k.llm
	default:
		return k.book
	}
}

// screenClasses returns a screen's fixed key classes.
func (k *keymap) screenClasses(s screen) []classDoc {
	switch s {
	case screenNews:
		return k.newsClasses
	case screenLLM:
		return k.llmClasses
	default:
		return k.bookClasses
	}
}

// lookup resolves a key on a screen: the screen's table first, then the
// globals (so a screen can shadow a global if it must — none does today).
func (k *keymap) lookup(s screen, key string) action {
	for _, b := range k.screenBindings(s) {
		if b.matches(key) {
			return b.action
		}
	}
	for _, b := range k.global {
		if b.matches(key) {
			return b.action
		}
	}
	return actNone
}

func (b binding) matches(key string) bool {
	if b.key == key {
		return true
	}
	for _, alt := range b.alts {
		if alt == key {
			return true
		}
	}
	return false
}

// ApplyOverrides rebinds verb keys from an {"action":"key"} map
// (RSX_TERM_KEYMAP). Unknown actions and collisions with a key already bound
// on the same screen (or a fixed class key) are rejected — a broken keymap
// must fail loudly at startup, not silently swallow a trading key.
func (k *keymap) ApplyOverrides(overrides map[string]string) error {
	tables := [][]binding{k.global, k.book, k.news, k.llm}
	for name, key := range overrides {
		if key == "" {
			return fmt.Errorf("keymap: action %q bound to an empty key", name)
		}
		found := false
		for _, table := range tables {
			for i := range table {
				if table[i].action == action(name) {
					table[i].key = key
					found = true
				}
			}
		}
		if !found {
			return fmt.Errorf("keymap: unknown action %q", name)
		}
	}
	return k.checkConflicts()
}

// checkConflicts rejects two actions sharing a key within one screen's
// effective map (screen + globals).
func (k *keymap) checkConflicts() error {
	for _, s := range []screen{screenBook, screenNews, screenLLM} {
		seen := map[string]action{}
		effective := append(append([]binding{}, k.screenBindings(s)...), k.global...)
		for _, b := range effective {
			for _, key := range append([]string{b.key}, b.alts...) {
				if prev, dup := seen[key]; dup && prev != b.action {
					return fmt.Errorf("keymap: %q bound to both %q and %q on the %s screen", key, prev, b.action, s.label())
				}
				seen[key] = b.action
			}
		}
	}
	return nil
}

// hintFor renders a screen's persistent hint line from the table: the
// hint-flagged bindings, screen-local first, then globals.
func (k *keymap) hintFor(s screen) string {
	var parts []string
	for _, b := range k.screenBindings(s) {
		if b.hint != "" {
			parts = append(parts, hintLabel(b))
		}
	}
	for _, b := range k.global {
		if b.hint != "" {
			parts = append(parts, hintLabel(b))
		}
	}
	return " " + strings.Join(parts, "  ") + " "
}

// hintLabel keeps a rebound key honest on the hint line: the label's leading
// key token is replaced when the primary key was overridden.
func hintLabel(b binding) string {
	fields := strings.Fields(b.hint)
	if len(fields) > 1 && !strings.Contains(fields[0], b.key) {
		fields[0] = b.key
		return strings.Join(fields, " ")
	}
	return b.hint
}

// helpLines renders the ? overlay body for a screen: the globals, the
// screen's verbs (rebound keys shown as bound), and its fixed key classes.
// Generated from the same table that dispatches — never hand-maintained.
func (k *keymap) helpLines(s screen) []string {
	row := func(keys, help string, danger bool) string {
		style := StyleTextBright
		if danger {
			style = StyleAsk
		}
		return style.Render(fmt.Sprintf("  %-11s", keys)) + StyleMuted.Render(help)
	}
	// The single-key-fire caveat is a BOOK-screen truth only; the news and
	// assistant screens have no order keys, so their section header is plain.
	section := s.label()
	if s == screenBook {
		section += " (orders fire on ONE key — the size + notional caps still hard-block)"
	}
	out := []string{
		StyleHeading.Bold(true).Render("KEYS — " + s.label() + " · any key to close"),
		"",
		StyleHeading.Render(section),
	}
	for _, b := range k.screenBindings(s) {
		out = append(out, row(keysLabel(b), b.help, b.danger))
	}
	for _, c := range k.screenClasses(s) {
		out = append(out, row(c.keys, c.help, c.danger))
	}
	out = append(out, "", StyleHeading.Render("everywhere"))
	for _, b := range k.global {
		out = append(out, row(keysLabel(b), b.help, b.danger))
	}
	out = append(out, "", StyleMuted.Render("  rebind verbs via RSX_TERM_KEYMAP (JSON {\"action\":\"key\"}); actions:"))
	out = append(out, StyleMuted.Render("  "+strings.Join(k.actionNames(), " ")))
	return out
}

// keysLabel renders a binding's key plus alternates ("h / ←").
func keysLabel(b binding) string {
	keys := []string{b.key}
	keys = append(keys, b.alts...)
	return strings.Join(keys, " ")
}

// actionNames lists every rebindable action, sorted and de-duplicated, for the
// help footer (an action bound on more than one screen — e.g. back — appears
// once).
func (k *keymap) actionNames() []string {
	seen := map[string]bool{}
	var names []string
	for _, table := range [][]binding{k.global, k.book, k.news, k.llm} {
		for _, b := range table {
			if name := string(b.action); !seen[name] {
				seen[name] = true
				names = append(names, name)
			}
		}
	}
	sort.Strings(names)
	return names
}
