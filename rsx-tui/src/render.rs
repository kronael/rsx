//! Rendering. Pure over `&App` — draws into whatever backend the
//! caller's `Frame` wraps (a real terminal in the binary, a
//! `TestBackend` in tests).

use crate::app::App;
use crate::app::Field;
use crate::conn::Side;
use ratatui::layout::Constraint;
use ratatui::layout::Direction;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Cell;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_status(f, root[0], app);
    draw_speed(f, root[2], app);
    draw_statusline(f, root[3], app);
    draw_help(f, root[4]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(root[1]);

    draw_book(f, cols[0], app);
    draw_order_entry(f, cols[1], app);
    draw_right(f, cols[2], app);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let conn = if app.connected {
        Span::styled("● live", Style::default().fg(Color::Green))
    } else {
        Span::styled(
            "● offline",
            Style::default().fg(Color::Yellow),
        )
    };
    let line = Line::from(vec![
        Span::styled(
            format!(" RSX  {} ", app.symbol),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        conn,
        Span::styled(
            format!("   open {}   fills {}", app.open_orders, app.fills),
            Style::default().fg(Color::Gray),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

/// Format a nanosecond duration adaptively with integer math (no
/// floats): `340 ns`, `9.6 µs`, `1.28 ms`.
fn fmt_ns(ns: u64) -> String {
    if ns < 1_000 {
        format!("{ns} ns")
    } else if ns < 1_000_000 {
        format!("{}.{} µs", ns / 1_000, (ns % 1_000) / 100)
    } else {
        format!("{}.{:02} ms", ns / 1_000_000, (ns % 1_000_000) / 10_000)
    }
}

/// The speed strip — the whole point of the terminal's pitch. Shows the
/// last round-trip split into net / internal / engine, plus rolling
/// p50 and best. Dim until the first measurement arrives.
fn draw_speed(f: &mut Frame, area: Rect, app: &App) {
    let cyan = Style::default().fg(Color::Cyan);
    let dim = Style::default().fg(Color::DarkGray);
    let spans = match app.last_lat {
        None => vec![Span::styled(
            " ⚡ latency: waiting for first round-trip…",
            dim,
        )],
        Some(l) => {
            let total = fmt_ns(l.total_ns());
            let p50 = app.lat_p50_ns().map(fmt_ns).unwrap_or_default();
            let best = app.lat_min_ns().map(fmt_ns).unwrap_or_default();
            vec![
                Span::styled(
                    format!(" ⚡ RTT {total} "),
                    cyan.add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(
                        "= net {} + internal {} + engine {}",
                        fmt_ns(l.net_ns),
                        fmt_ns(l.internal_ns),
                        fmt_ns(l.engine_ns),
                    ),
                    Style::default().fg(Color::Gray),
                ),
                Span::styled(
                    format!("   p50 {p50} · best {best}"),
                    dim,
                ),
            ]
        }
    };
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The most recent event / action, so the trader gets feedback.
fn draw_statusline(f: &mut Frame, area: Rect, app: &App) {
    let line = Line::from(Span::styled(
        format!(" {}", app.status),
        Style::default().fg(Color::White),
    ));
    f.render_widget(Paragraph::new(line), area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let line = Line::from(Span::styled(
        " q quit  b/s side  t tif  tab field  0-9 type  \
         ⌫ del  enter submit ",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(line), area);
}

/// Orderbook ladder: asks (red) top-down to the spread, bids (green)
/// below. Depth bar width scales with qty. Empty until the first Book
/// event arrives.
fn draw_book(f: &mut Frame, area: Rect, app: &App) {
    let mut rows: Vec<Row> = Vec::new();
    // Asks worst-first so the best ask sits just above the spread.
    for (px, qty) in app.asks.iter().rev() {
        rows.push(level_row(*px, *qty, Color::Red));
    }
    rows.push(Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled(
            format!("— {} —", app.spread()),
            Style::default().fg(Color::DarkGray),
        )),
        Cell::from(""),
    ]));
    for (px, qty) in &app.bids {
        rows.push(level_row(*px, *qty, Color::Green));
    }
    let table = Table::new(
        rows,
        [
            Constraint::Length(9),
            Constraint::Length(7),
            Constraint::Min(0),
        ],
    )
    .block(Block::default().borders(Borders::ALL).title(" book "));
    f.render_widget(table, area);
}

fn level_row(px: i64, qty: i64, color: Color) -> Row<'static> {
    let bar = "▊".repeat((qty as usize).min(30));
    Row::new(vec![
        Cell::from(Span::styled(px.to_string(), Style::default().fg(color))),
        Cell::from(qty.to_string()),
        Cell::from(Span::styled(bar, Style::default().fg(color))),
    ])
}

fn draw_order_entry(f: &mut Frame, area: Rect, app: &App) {
    let e = &app.entry;
    let (buy, sell) = match e.side {
        Side::Buy => (Modifier::REVERSED, Modifier::empty()),
        Side::Sell => (Modifier::empty(), Modifier::REVERSED),
    };
    let price = field_span("price", &e.price, e.focus == Field::Price);
    let qty = field_span("qty  ", &e.qty, e.focus == Field::Qty);
    let body = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "  BUY  ",
                Style::default().fg(Color::Green).add_modifier(buy),
            ),
            Span::raw("  "),
            Span::styled(
                "  SELL  ",
                Style::default().fg(Color::Red).add_modifier(sell),
            ),
        ]),
        Line::from(""),
        price,
        qty,
        Line::from(format!("  tif:   {}", e.tif.label())),
        Line::from(""),
        Line::from(Span::styled(
            "  enter → submit",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(body)
        .block(Block::default().borders(Borders::ALL).title(" order "));
    f.render_widget(p, area);
}

/// A labelled field; the focused one shows a cursor and is bold.
fn field_span(label: &str, value: &str, focused: bool) -> Line<'static> {
    let shown = if focused {
        format!("  {label}: {value}_")
    } else {
        format!("  {label}: {value}")
    };
    let style = if focused {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };
    Line::from(Span::styled(shown, style))
}

fn draw_right(f: &mut Frame, area: Rect, app: &App) {
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    draw_positions(f, split[0], app);
    draw_trades(f, split[1], app);
}

fn draw_positions(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app
        .positions
        .iter()
        .map(|(sym, net, entry, upnl)| {
            let pnl_color =
                if *upnl >= 0 { Color::Green } else { Color::Red };
            Row::new(vec![
                Cell::from(*sym),
                Cell::from(net.to_string()),
                Cell::from(entry.to_string()),
                Cell::from(Span::styled(
                    upnl.to_string(),
                    Style::default().fg(pnl_color),
                )),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Min(8),
            Constraint::Length(6),
            Constraint::Length(7),
            Constraint::Length(7),
        ],
    )
    .header(
        Row::new(vec!["sym", "net", "entry", "upnl"])
            .style(Style::default().fg(Color::DarkGray)),
    )
    .block(Block::default().borders(Borders::ALL).title(" positions "));
    f.render_widget(table, area);
}

fn draw_trades(f: &mut Frame, area: Rect, app: &App) {
    let lines: Vec<Line> = app
        .trades
        .iter()
        .map(|(side, px, qty)| {
            let color = match side {
                Side::Buy => Color::Green,
                Side::Sell => Color::Red,
            };
            Line::from(vec![
                Span::styled(
                    format!("{px:>7}"),
                    Style::default().fg(color),
                ),
                Span::raw(format!("  {qty}")),
            ])
        })
        .collect();
    let p = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" trades "));
    f.render_widget(p, area);
}
