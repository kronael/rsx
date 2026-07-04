# Playground restyle: left accent bars → surrounding accents (krons style)

Directive: remove ALL left accent bars in the playground; convert to
**surrounding accents** matching krons.fiu.wtf. Keep the semantic colors.
Make them good.

## krons source of truth (krons.fiu.wtf/assets/hub.css)
`:root` — `--accent:#60a5fa` (blue) · `--accent2:#fbbf24` (amber) ·
`--accent3:#7dd3fc` (cyan) · `--red:#f87171` · `--border:#1a2332` ·
`--card:#151b28` · `--dim:#a0a8b8` · `--bg:#0a0e1a` · `--fg:#f5f5f0`.
**Gold `#d4af37` is OFF-BRAND — never use.**

krons box shapes (all `border-radius:3px`):
- `.tldr-box` — **2px solid all-sides**, `var(--accent)` blue, subtle
  gradient bg, pad 1.5em. → the PROMINENT surrounding accent.
- `.insight` — **1px solid all-sides**, `var(--accent2)` amber, gradient
  bg, pad 1em 1.2em. → a subtle typed surrounding box.
- `.card` — 1px solid all-sides `var(--border)`, `var(--card)` bg.
- `.callout` — 3px LEFT only (we are NOT copying this; the user wants
  surrounding, not left).

## Palette maps 1:1 to the existing playground semantic palette
Keep colors; only the border SHAPE changes (left bar → full ring, 3px radius).
| meaning | playground colour | krons equiv |
|---|---|---|
| info / hint / link | blue-500 / text-blue-400 (#60a5fa) | --accent |
| warning / degraded | amber-500 | --accent2 |
| error / reject | red-500 (#f87171) | --red |
| success / live | emerald-500 | (keep playground's) |
| accent / heading | cyan-400 | --accent3 |

## The exact Tailwind translation (10 sites, keep `bg-slate-800/50`)
- `border-l-4 border-{c}-500` (prominent) → `border-2 border-{c}-500/70 rounded-[3px]`
- `border-l-2 border-{c}-500` (subtle)    → `border border-{c}-500/60 rounded-[3px]`
- keep the semantic `{c}`, keep `bg-slate-800/50`, keep the text.
- Markdown blockquotes (`border-left:Npx solid #334155` in server.py:2817,
  pages.py:5588) → `border:1px solid #334155; border-radius:3px;
  padding:.5rem .75rem;` (surrounding, dim `#94a3b8`, keep italic-free).

## Sites (grep `border-l-[0-9]|border-left` in rsx-playground/*.py)
server.py:2817 · pages.py:257,516,3447,3454,4744,5588,5703 ·
cast_demo.py:265,273. (10 total; `_hint_bar` @ pages.py:5703 is the
canonical `border-l-4 border-blue-500` — fix it and copy the new shape.)

## Also update the design-system doc
`rsx-playground/CLAUDE.md` currently codifies "The one visual rule: LEFT
BARS … NEVER full-ring boxes." This directive REVERSES that. Rewrite that
section to "SURROUNDING ACCENTS" (full ring, `border-2` prominent /
`border` subtle, `rounded-[3px]`, semantic colour carries meaning, on
`bg-slate-800/50`; cite krons `.tldr-box`/`.insight`). Keep the palette
table unchanged (colours don't change). Keep the Playwright-green rule.

## Verify
Playwright gate stays green (nav/tab/redirect asserts shouldn't touch
border classes, but re-run gate-4 after). Visual pass on mobile too (this
folds into the docs-mobile-broken fix).
