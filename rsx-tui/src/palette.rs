//! Terminal palette — the exchange's shared "Ayam Cemani" colours.
//!
//! One place mapping meaning → `ratatui` RGB, mirroring the dashboard's
//! Tailwind retune (`rsx-playground/pages.py`) and its semantics
//! (`rsx-playground/CLAUDE.md`): a green-tinged near-black base, a neon
//! beetle-green for live/filled, a violet feather-sheen for headings, and
//! red/amber keeping their meaning. Colour is meaning, never decoration —
//! add a const only for a new *meaning*, never to "look nice".

use ratatui::style::Color;

/// Live / long / bid / filled — the neon beetle-green.
pub const LIVE: Color = Color::Rgb(0x22, 0xf5, 0xa1);
/// Bid side of the book (same green as live).
pub const BID: Color = LIVE;
/// Short / ask / down / reject.
pub const ASK: Color = Color::Rgb(0xf8, 0x71, 0x71);

/// Section heading / badge / the ⚡ speed motif — the violet sheen.
pub const HEADING: Color = Color::Rgb(0xbd, 0x83, 0xff);
/// Info / secondary accent (the lighter violet).
pub const ACCENT: Color = Color::Rgb(0xa9, 0x92, 0xff);
/// Overlay ring (the darker violet), e.g. the trace HUD border.
pub const RING: Color = Color::Rgb(0x7c, 0x3a, 0xed);

/// Body text.
pub const TEXT: Color = Color::Rgb(0xa9, 0xbc, 0xb2);
/// Bright text — focused field, active status line.
pub const TEXT_BRIGHT: Color = Color::Rgb(0xe7, 0xee, 0xea);
/// Muted — labels, captions, help, dim/secondary.
pub const MUTED: Color = Color::Rgb(0x58, 0x6b, 0x62);

/// Degraded / stale / offline — the warning amber.
pub const DEGRADED: Color = Color::Rgb(0xfb, 0xbf, 0x24);

/// Panel background.
pub const PANEL_BG: Color = Color::Rgb(0x0d, 0x17, 0x12);
/// Page background (darkest slate).
pub const PAGE_BG: Color = Color::Rgb(0x04, 0x08, 0x06);
/// Panel border.
pub const BORDER: Color = Color::Rgb(0x16, 0x21, 0x1b);
