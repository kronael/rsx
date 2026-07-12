# Keycast — the on-screen keypress overlay (RSX_TERM_KEYCAST=1)

## The problem

A keyboard-only terminal is invisible on film. In a demo GIF or a teaching
session the screen changes — cursor jumps, an order rests, a row freezes —
but the *cause* (which key did that) is off-camera. Screen-recorder keycast
tools (screenkey, KeyCastr) exist, but they composite over the whole desktop:
they can't ride an SSH session, don't survive an asciinema→GIF pipeline, and
know nothing about what the key *means* in this app.

## The fix

An env-gated overlay inside the terminal itself: `RSX_TERM_KEYCAST=1` shows
each pressed key as a small chip in the bottom-right corner — the key, plus
the action it dispatched, e.g. `⌨ f  place resting limit at the cursor`.

Three deliberate choices:

- **The label is read from the dispatch table** (`ui/keymap.go`), never a
  second key→label map — so the chip can only ever say what the key actually
  does, and a rebound key (`RSX_TERM_KEYMAP`) casts its real binding. Fixed
  key classes (size presets, crosses, symbol letters) label from their
  `classDoc` entries; in a capture mode (symbol switcher, venue picker, news
  search, live chat typing) the chip shows the **bare key** — labelling `f`
  as "place" while it types into the chat would lie (see `honesty.md`).
- **Pure fold, pure render.** The chip is a tiny struct (`key`, `label`,
  `ttl`) folded on `tea.KeyMsg` and aged by the heatmap's existing ~100 ms
  bin tick — no `time.Now()` in the render, so the view stays a pure
  function of the model (the UI-agnostic keeper) and the fade is testable
  tick by tick. In the DOM view (no bin tick) a chip simply persists until
  the next key.
- **Byte-identical when off.** Off (the default), nothing is recorded and
  the overlay renders empty — the frame is exactly today's, which is what
  the DOM/book goldens lock. When on, the chip right-aligns onto the lowest
  footer line with room (never wrapping a line — that would shift the fixed
  grid).

## The cost it removes

Demos and teaching no longer need an out-of-band keystroke display (or a
narrator): every recording made over plain asciinema/tmux/SSH carries its own
"what did they press" track, and the labels can never drift from the keymap
because they *are* the keymap.

---

*Related: the one-table keymap pattern (dispatch = hint line = `?` help =
keycast label) is described in `ui/keymap.go`'s header; the fade rides the
bin tick described in `compression.md`.*
