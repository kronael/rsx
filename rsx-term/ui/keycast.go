package ui

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// Keycast (RSX_TERM_KEYCAST=1) is the on-screen keypress overlay for demos
// and teaching: the key just pressed, plus what it does, as a small chip in
// the bottom-right corner. The label comes from the SAME keymap table that
// dispatches the key (keymap.go) — never a second hand-maintained key→label
// map, so it can't drift. Off (the default) the render is byte-identical:
// the chip only exists while a recent key's ttl is positive, and nothing is
// ever recorded. See notes/keycast.md.

// keycastTTL is how many bin ticks (~100 ms each) a chip stays up (~2 s);
// the last keycastFade ticks render muted — the fade-out. The clock is the
// heatmap's existing bin tick, not a render-time time.Now() (keeper: folds
// are clock-free, the view is a pure function of the model). The DOM view
// has no bin tick, so there a chip simply persists until the next key.
const (
	keycastTTL  = 20
	keycastFade = 6
)

// keycast is the overlay's whole state: the last key, its keymap label, and
// the ticks it has left. A plain fold-owned struct — observe on tea.KeyMsg,
// tick on binTickMsg, render via Model.keycastChip.
type keycast struct {
	key   string
	label string
	ttl   int
}

// observe records a keypress.
func (kc *keycast) observe(key, label string) {
	kc.key = key
	kc.label = label
	kc.ttl = keycastTTL
}

// tick ages the chip by one bin tick.
func (kc *keycast) tick() {
	if kc.ttl > 0 {
		kc.ttl--
	}
}

// keycastLabel resolves what the chip says a key does: the dispatching
// binding's help on the active screen, else the fixed key-class doc (size
// presets, crosses, symbol letters). In a capture mode — help overlay,
// symbol switcher, venue picker, news search, live chat typing — keys are
// raw input, not verbs, so the label is empty and the chip shows the bare
// key (labelling "f" as "place" while it types into the chat would lie).
// The DOM view dispatches its own hardcoded switch, not the table: bare
// keys there too.
func (m Model) keycastLabel(key string) string {
	if !m.cfg.Stream || m.showHelp || m.switching || m.venuePicking {
		return ""
	}
	if m.screen == screenNews && m.newsSearch {
		return ""
	}
	if m.screen == screenLLM && m.assist.Enabled() && key != "tab" && key != "shift+tab" {
		return ""
	}
	if b, ok := m.keys.bindingFor(m.screen, key); ok {
		return chipLabel(b.help)
	}
	for _, c := range m.keys.screenClasses(m.screen) {
		if classKeyMatches(c.keys, key) {
			return chipLabel(c.help)
		}
	}
	return ""
}

// classKeyMatches reports whether a pressed key falls inside a classDoc's
// keys spec — the shipped class grammars: "1-5" (digit presets), "⇧1-5"
// (the shifted digit row = crosses), "a-z" (symbol letters). The chat
// "type" class never labels a chip.
func classKeyMatches(spec, key string) bool {
	if len(key) != 1 {
		return false
	}
	c := key[0]
	switch spec {
	case "1-5":
		return c >= '1' && c <= '5'
	case "⇧1-5":
		return strings.ContainsAny(key, "!@#$%")
	case "a-z":
		return c >= 'a' && c <= 'z'
	}
	return false
}

// chipLabelCap bounds the chip's label so it stays a corner chip, never a
// second status line.
const chipLabelCap = 40

// chipLabel compacts a keymap help string for the chip: the trailing
// parenthetical detail is dropped ("cursor to an older row (freeze, not
// replay)" → "cursor to an older row") and the rest is rune-capped.
func chipLabel(help string) string {
	if i := strings.Index(help, " ("); i > 0 {
		help = help[:i]
	}
	runes := []rune(help)
	if len(runes) > chipLabelCap {
		return string(runes[:chipLabelCap-1]) + "…"
	}
	return help
}

// keycastKey renders the key token itself: arrows as their glyphs, the
// space bar visibly, everything else verbatim ("enter", "f9", "shift+tab").
func keycastKey(key string) string {
	switch key {
	case "up":
		return "↑"
	case "down":
		return "↓"
	case "left":
		return "←"
	case "right":
		return "→"
	case " ":
		return "␣"
	}
	return key
}

// keycastChip renders the overlay chip, or "" when the overlay is off or the
// last key has aged out — the byte-identical-when-off guarantee. Fresh: the
// key token as an accent block (the mode-line tag treatment) with the label
// beside it; fading: the whole chip drops to muted.
func (m Model) keycastChip() string {
	kc := m.keycast
	if !m.cfg.Keycast || kc.key == "" || kc.ttl <= 0 {
		return ""
	}
	key := "⌨ " + keycastKey(kc.key)
	if kc.ttl <= keycastFade {
		out := key
		if kc.label != "" {
			out += "  " + kc.label
		}
		return StyleMuted.Render(" " + out + " ")
	}
	chip := lipgloss.NewStyle().
		Foreground(ColorPageBg).
		Background(ColorAccent).
		Bold(true).
		Render(" " + key + " ")
	if kc.label != "" {
		chip += StyleTextBright.Render(" " + kc.label + " ")
	}
	return chip
}

// chipMaxClimb is how many lines up from the bottom the chip may land: the
// footer's status/latency/hint block. Past that it is no longer a corner
// chip, so a frame with no room there just goes without.
const chipMaxClimb = 3

// overlayBottomRight right-aligns chip onto the lowest of the render's last
// chipMaxClimb lines with room for it, padded to the terminal width — the
// footer lines are left-aligned, so the chip lands on otherwise-empty cells
// (a full hint line just pushes it one line up). No line is ever extended
// past width or wrapped (that would shift the whole fixed grid), and an
// empty chip or unknown width returns the frame unchanged.
func overlayBottomRight(view, chip string, width int) string {
	if chip == "" || width <= 0 {
		return view
	}
	lines := strings.Split(view, "\n")
	for up := 1; up <= chipMaxClimb && up <= len(lines); up++ {
		i := len(lines) - up
		pad := width - lipgloss.Width(lines[i]) - lipgloss.Width(chip)
		if pad < 1 {
			continue
		}
		lines[i] += strings.Repeat(" ", pad) + chip
		return strings.Join(lines, "\n")
	}
	return view
}
