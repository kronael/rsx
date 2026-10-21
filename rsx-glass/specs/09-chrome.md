---
status: draft
---

# 09 — Shared chrome

The chrome is everything every screen shares around its body: the header,
the mode line that says what the next key will do, the status line, the
generated hint line, the `?` help overlay, the keycast chip, the symbol
switcher and the venue picker. Screen bodies are in files 04-08; key
grammars in [10-keymap.md](10-keymap.md); acceptance in
[17-acceptance.md](17-acceptance.md) § CHROME.

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## Header

Line 1 of every streaming screen: badge ` RSX  <active instrument> ` (page ink
on Heading, bold) · link dot (`● live` Live / `● offline` Degraded) ·
`◀ bids  mid <px>  asks ▶` [code rsx-term/ui/stream.go:832-852]. On NEWS,
LLM and USERS the header still describes the BOOK's active market. Glass
requirements on it: BK-02, BK-03, BK-04 in [04-screen-book.md](04-screen-book.md).

## Mode line

Line 2 of every streaming screen — the safety surface that tells the trader
what mode the next key fires in [founder F-KEY-03 "reduce only are modifiers..
toggled ... displayed maybe at the top.of the screen"]:

```go
// rsx-term/ui/stream.go:576-611 (exact)
func (m Model) modeLine() string {
	tag := lipgloss.NewStyle().
		Foreground(ColorPageBg).
		Background(ColorAccent).
		Bold(true).
		Render(" " + m.screen.label() + " ")
	mod := func(name string, on bool) string {
		if on {
			return StyleArmed.Render(" " + name + " ")
		}
		return StyleMuted.Render(" " + name + " off ")
	}
	venue := m.activeVenue
	if m.screen == screenNews {
		venue = m.watchVenue()
	}
	parts := []string{
		tag,
		StyleHeading.Render(" " + venue + " "),
		mod("RO", m.reduceOnly),
		mod("PO", m.postOnly),
		StyleMuted.Render(fmt.Sprintf(" size %s [%d] ", m.fmtQty(m.sizePreset()), m.sizeSel+1)),
		StyleMuted.Render(" list " + m.lists[m.listSel].name + " "),
	}
	if m.switching {
		parts = append(parts, StyleTextBright.Bold(true).Render(" switch: "+m.switchBuf+"_ ")+m.switchCandidates())
	}
	if m.venuePicking {
		var vs []string
		for _, v := range m.venues {
			vs = append(vs, v.Code+" "+v.Name)
		}
		parts = append(parts, StyleTextBright.Bold(true).Render(" venue? ")+StyleMuted.Render(strings.Join(vs, " · ")))
	}
	return lipgloss.JoinHorizontal(lipgloss.Top, parts...)
}
```

Rendered at the pin:

```
 BOOK  rsx  RO off  PO off  size 1.0000 [1]  list rsx  switch: _ e PENGU-PERP · t BTC-PERP · h ETH-PERP · o SOL-PERP · a XRP-PERP
 BOOK  rsx  RO off  PO off  size 1.0000 [1]  list hyperliquid  venue? r rsx · h hyperliquid
```

Screen tags: `BOOK`, `NEWS`, `LLM`, `USERS`. An ON modifier draws in the
Armed style (red ground) so it cannot be missed. The switcher lists the active
venue's codes matching the buffer, at most 8 then `…`; no match → Ask
`no match` [code rsx-term/ui/stream.go:613-632].

| ID | Requirement | Source |
|---|---|---|
| CH-01 | The mode line is visible on every screen that can send an order, and shows every persistent modifier that changes what an order key sends (RO, PO, size preset, venue, and ARMED where the confirm exists). | [code rsx-term/ui/stream.go:572-575], [founder F-KEY-03] |
| CH-02 | The mode line shows the venue the next order goes to; on NEWS it shows the breadth venue, so on NEWS it is NOT the order venue — glass labels which is which. | [code rsx-term/ui/stream.go:588-591] |

## Status line and hint line

The status line is one TextBright line with the result of the last action
or event (`sent …`, `BLOCKED: …`, `order 7 done`, `fill 7: 14.0000 @ 0.009998`,
`rejected: …`, `venue → hyperliquid`, `book → WIF-PERP`, `microscope off`,
`<venue> feed down (reconnecting)`). Initial value `connecting…` (or the
keymap rejection) [code rsx-term/ui/model.go:415-421]. Every keypress that does
something writes it [founder F-KEY-09]. BOOK lacks a drawn status line at the
pin (BK-10).

The hint line is generated from the keymap: the screen's hint-flagged bindings
then the globals' [code rsx-term/ui/keymap.go:279-305]. Hints at the pin:

| Screen | Hint line |
|---|---|
| BOOK | ` x symbol  n news  b/s side  h/l cursor  j/k touch  f place  d cancel  q quit  tab view  r RO  ? help ` |
| NEWS | ` / search  j/k select  enter → assistant  esc back  q quit  tab view  r RO  ? help ` |
| LLM | ` esc → news  q quit  tab view  r RO  ? help ` |
| USERS | ` / search  j/k select  enter detail  g refresh  o role  a adjust  z freeze  esc back  q quit  tab view  r RO  ? help ` |

| ID | Requirement | Source |
|---|---|---|
| CH-10 | A compact key legend at the top right of every screen, generated from the same hint fields. | [founder F-KEY-06], [doc specs/2/60-terminal-assistant.md "Key legend, every screen, top-right"] |
| CH-11 | The hint shows only keys that act in the current mode (US-04). | |
| CH-12 | `p PO` appears in hints if `r RO` does (PO is an equally persistent order modifier and is absent from every hint at the pin). | [code rsx-term/ui/keymap.go:109-110] |

## Help overlay

`?` replaces the screen with a Ring panel generated per screen: title
`KEYS — <SCREEN> · any key to close`; the screen section (on BOOK titled
`BOOK (orders fire on ONE key — the size + notional caps still hard-block)`);
each binding `  <keys>  <help>` with danger rows in Ask; the screen's key
classes; `everywhere` with the globals; the rebind note and the sorted action
list. Any key closes it; `q`/`ctrl+c` quit [code rsx-term/ui/keymap.go:307-349],
[code rsx-term/ui/stream.go:689-693]. Glass: US-08 (fit the window), KM-22
(also on the order-entry view).

## Keycast

An opt-in overlay (`RSX_TERM_KEYCAST=1`) that shows each pressed key and what
it did, for demos and teaching, labelled from the dispatching table so it can
never lie about a binding [founder F-KEY-07],
[doc rsx-term/notes/keycast.md].

```go
// rsx-term/ui/keycast.go:22-48, 136-189 (exact, comments trimmed)
const (
	keycastTTL  = 20
	keycastFade = 6
)

type keycast struct {
	key   string
	label string
	ttl   int
}

func (kc *keycast) observe(key, label string) {
	kc.key = key
	kc.label = label
	kc.ttl = keycastTTL
}

func (kc *keycast) tick() {
	if kc.ttl > 0 {
		kc.ttl--
	}
}

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

const chipMaxClimb = 3

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
```

Rules: a chip lives 20 bin ticks (~2 s) and fades to Muted for the last 6;
the clock is the model's 100 ms bin tick, never a render-time clock; the key
token shows arrows as `↑ ↓ ← →` and space as `␣`; the label is the binding's
help with a trailing parenthetical dropped, capped at 40 runes; a capture-mode
key shows bare (H-10); off, the frame is byte-identical
[code rsx-term/ui/keycast.go:50-133]. Glass keeps the chip (founder request;
the element inventory tagged it cuttable for an OS-level keycast — the
founder's request wins), drawn bottom-right over the footer, never shifting
the grid.

## Walkthrough tooltips

CH-20: an in-UI walkthrough built from ELI13 tooltips that reuse the docs,
organised like psr.marinade.finance's docs and tooltips, for power users
[founder F-DEM-03]. Not built in the terminal; the repo docs a tooltip would
reuse describe the terminal, not glass, and are rewritten first. Scope and
placement: OD-14.

## Symbol switcher and venue picker

Grammar: [10-keymap.md](10-keymap.md) § capture modes. The venue picker opens
only when more than one venue is configured; the primary's letter is its
first letter (`r` for rsx), the extra venues' letters come from their config
(`h` Hyperliquid, `p` Phoenix) [code rsx-term/ui/model.go:437, 455-461],
[code rsx-term/main.go:165-170, 212-217].
Switching a venue lands on that venue's last active symbol, else its first
instrument [code rsx-term/ui/update.go:367-388].
