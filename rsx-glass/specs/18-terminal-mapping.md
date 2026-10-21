---
status: reference
---

# 18 — Terminal mechanisms and their glass equivalents

This file lists every mechanism that existed only because rsx-term ran in a
text terminal (ANSI output, a PTY, a character grid, terminal key strings,
the terminal-recording demo pipeline, the SSH and web-PTY access paths) and
states what replaces each in glass, so nothing the terminal did silently
disappears with the medium. What glass already implements is
[19-glass-status.md](19-glass-status.md).

Source pin: citations are at `5d175b6ff04755e02fcca5ebbb918b01b929c23b`.

## Runtime and rendering

| Terminal mechanism | What it gave | Glass equivalent | Source |
|---|---|---|---|
| Bubble Tea Elm loop: `Init / Update(msg) / View()`; every event (`tea.KeyMsg`, wire frames, link events, ticks, command results) is a message folded by one goroutine | a single-owner model, pure folds, a render that is a pure function of the model | one folding goroutine owns the model (`glass.Feed` pattern); a separate render loop presents the newest `Grid` (`glass.Run`); data rate ⟂ render rate | [code rsx-term/ui/update.go:19-186], [code rsx-glass/glass.go], [code rsx-glass/feed.go] |
| `tea.Tick(100ms)` bin tick | heatmap bins and the keycast clock | a 100 ms ticker in the fold goroutine (`cmd/glass` already has `binTick`) | [code rsx-term/ui/stream.go:152-164], [code rsx-glass/cmd/glass/main.go:43-48] |
| `tea.Cmd` for async work (support HTTP calls) | off-loop requests whose results fold back as messages | named goroutines whose results enter the fold channel as messages | [code rsx-term/ui/users_view.go:99-177] |
| `drainEvents` pumps (one per source) | transport goroutines feed the program | the same: connectors emit on channels, one pump forwards into the fold | [code rsx-term/main.go:700-706] |
| alt-screen, line-diffed repaint | a fixed grid, no scrollback | a full-frame `Grid` per bin, presented in place | [code rsx-term/main.go:374], DS-30 |
| `tea.WindowSizeMsg` | the grid re-derived from the window; history survives resize | derive the `Grid` size from the framebuffer and the atlas cell size (glass v1 letterboxes a fixed 72 × 24: OD-23) | [code rsx-term/ui/update.go:171-176, 733-748] |
| lipgloss styles → ANSI SGR (fg, bg, bold, italic, reverse) | colour and emphasis per cell | resolved `Cell{Bg, Fg, Glyph}` RGBA; bold and italic need atlas variants or an equivalent (the derived style is italic: DS-20) | [code rsx-term/ui/styles.go], [code rsx-glass/grid.go] |
| lipgloss panels (`NormalBorder`), `JoinHorizontal/Vertical`, width padding | panel frames and column layout | box drawing in the grid, or spacing (border tagged cuttable) | [code rsx-term/ui/view.go:77-83] |
| render-tier ladder (`RSX_TERM_COLOR`, `NO_COLOR`, `TERM`, `COLORTERM`) | true colour → 16 colour → plain | none: always full colour; the colourblind theme stays; shape redundancy is tested by desaturation (AC-DS-04) | [code rsx-term/ui/stream.go:140-193] |
| glyph calibration per terminal font (`tools/glyphbank`, DejaVuSansMono coverage; braille excluded) | a perceptually ordered glyph set that renders on the user's font | the atlas draws heat glyphs as primitives (no font lottery); the measured ORDER of each ramp is kept | [code rsx-term/tools/glyphbank/glyphbank.py], [code rsx-glass/atlas/atlas.go] |
| ANSI-stripped text goldens | layout locks (colour invisible) | PNG goldens through `soft` (see colour) + the contrast matrix | [code rsx-term/ui/dom_golden_test.go] |
| `TestDumpScreens` (`RSX_TERM_DUMP=1` writes every screen as text) | a visual review gallery | a `soft` PNG gallery of every screen and state, behind an opt-in | [code rsx-term/ui/screendump_test.go] |
| width math with `len()` on ASCII and `lipgloss.Width` | aligned columns | cell counts in the grid (one rune = one cell; no double-width glyphs in the vocabulary) | [code rsx-term/ui/format.go:88-98] |

## Input

| Terminal mechanism | Glass equivalent | Source |
|---|---|---|
| `tea.KeyMsg.String()` names: `q`, `ctrl+c`, `tab`, `shift+tab`, `esc`, `enter`, `backspace`, `up`/`down`/`left`/`right`, `f2`/`f3`/`f9`, printable runes including `!@#$%`, `+`, `=`, `-`, `.`, `/`, `?`, `Z`, `X`, `R` | a key-event layer that produces the same names from GLFW key + modifier events and the character callback (shifted digits come from the character, not the key code), so the one keymap table and its JSON rebinds keep working unchanged | [code rsx-term/ui/keymap.go], [founder F-KEY-08] |
| mouse disabled (no `WithMouseCellMotion`) | no mouse handling beyond window controls (OD-01) | [code rsx-term commit 92bf9817] |
| keycast chip appended to a footer line | a chip drawn bottom-right over the footer cells | [code rsx-term/ui/keycast.go:136-189] |
| `ctrl+c` from the terminal | `ctrl+c` and the window close button quit | [code rsx-term/ui/keymap.go:105] |

## Access and distribution

| Terminal path | Status at the pin | Glass | Source |
|---|---|---|---|
| SSH forced-command dispatch (`restrict,pty,command="…rsx-tui-dispatch <user_id>"`, one restricted unix user, one line per registered key) | implemented for the earlier terminal; spec 54 | none (a GL window does not travel over SSH); the founder moved to "users will just run it on thier computers" | [doc specs/2/54-tui-access.md @ pin], [founder F-ACC-01] |
| playground `/terminal`: xterm.js over `/ws/terminal` spawning a PTY running `go run ./rsx-term` | implemented locally | removed with all web UIs | [doc specs/2/54-tui-access.md @ pin], [founder F-SURF-07] |
| `make term` (disabled: would dial the hosted deployment), `make term-local`, `make term-demo` (offline mock), `make term-smoke-llm` | Makefile targets | the same four shapes for glass: a disabled production target, local, offline demo, assistant smoke | [code Makefile:280-305] |
| a containerised client | asked for, not built | an optional container image of the glass binary | [founder F-ACC-01 "ideally also prpvide some containerized version"] |

Default endpoint. The terminal's code default is `ws://127.0.0.1:8088` (a local
gateway: `go run .` with no env dials it and exits if none is up); the offline
mock needs `RSX_GW_URL=mock` [code rsx-term/main.go:34-38, 91-95]. The founder's
rule is "Offline by default (`RSX_GW_URL=mock`)" [founder F-VEN-03]. Glass
already defaults to an in-process mock when `RSX_MD_URL` is empty
[code rsx-glass/cmd/glass/main.go:109-133]; SR-22: glass's default is the
offline mock for both channels.

## Demo pipeline

| Terminal step | Glass equivalent | Source |
|---|---|---|
| `walk.sh` drives the real binary in a headless tmux pane, polling the pane text to key off what is on screen | a driver that sends key events to the glass window and polls the model (or the soft render) for the same cues (`mid 0.`, `context handed off`, `⏳` gone) | [code rsx-term/demo/walk.sh] |
| asciinema `rec` → `agg` (github-dark, font 14, 118 × 33, idle limit 2 s) → `gifsicle -O3` | a frame capture of the window (or soft frames) encoded to GIF/MP4; tall-narrow variants for mobile | [code rsx-term/demo/Makefile], [founder F-DEM-04] |
| keycast on in every recording | the same | [founder F-KEY-07] |
| the scene: `playground demo-scene prep|run` (walls-profile book, +45% pump, a real liquidation, a winning demo trade), recorded fresh after an uncrossed-ladder check | unchanged; it is a playground tool | [doc rsx-term/demo/CLAUDE.md], [founder F-DEM-02] |
| README face: the trading-UI walkthrough; the dashboard walkthrough belongs to the playground | the glass walkthrough replaces the terminal GIF as the repo's face | [founder F-DEM-01] |
