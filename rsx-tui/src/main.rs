//! RSX trading terminal (ratatui).
//!
//! Scaffold: renders the full trade layout (orderbook ladder, order
//! entry, positions, trade tape, status bar) with MOCK data and a
//! working event loop. Live data is not wired yet — that step adds a
//! gateway WebSocket client (webproto 49) and reuses the workspace
//! wire types (rsx-types / rsx-messages). Run: `cargo run -p rsx-tui`.

use ratatui::crossterm::event;
use ratatui::crossterm::event::Event;
use ratatui::crossterm::event::KeyCode;
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
use ratatui::Terminal;
use std::io;
use std::time::Duration;

/// Which side a new order would hit. Cosmetic in the scaffold; drives
/// the order-entry highlight until the WS submit path lands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Buy,
    Sell,
}

/// Terminal UI state. Mock until the gateway WS client is wired.
struct App {
    symbol: &'static str,
    /// Ask levels, worst (highest) price first for top-down render.
    asks: Vec<(i64, i64)>,
    /// Bid levels, best (highest) price first.
    bids: Vec<(i64, i64)>,
    /// (side, price, qty) recent prints, newest first.
    trades: Vec<(Side, i64, i64)>,
    /// (symbol, net_qty, entry_px, upnl) open positions.
    positions: Vec<(&'static str, i64, i64, i64)>,
    side: Side,
    connected: bool,
}

impl App {
    fn mock() -> Self {
        App {
            symbol: "PENGU-PERP",
            asks: vec![
                (10_004, 12),
                (10_003, 8),
                (10_002, 20),
                (10_001, 5),
            ],
            bids: vec![
                (10_000, 7),
                (9_999, 15),
                (9_998, 9),
                (9_997, 30),
            ],
            trades: vec![
                (Side::Buy, 10_001, 5),
                (Side::Sell, 10_000, 3),
                (Side::Buy, 10_001, 11),
            ],
            positions: vec![("PENGU-PERP", 14, 9_998, 42)],
            side: Side::Buy,
            connected: false,
        }
    }
}

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

/// Event loop: redraw, then block up to 100ms for a key. `q`/Esc quit;
/// `b`/`s` toggle the order-entry side.
fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
) -> io::Result<()> {
    let mut app = App::mock();
    loop {
        terminal.draw(|f| draw(f, &app))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('b') => app.side = Side::Buy,
                KeyCode::Char('s') => app.side = Side::Sell,
                _ => {}
            }
        }
    }
}

fn draw(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_status(f, root[0], app);
    draw_help(f, root[2]);

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
            "● DEMO (no gateway)",
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
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let line = Line::from(Span::styled(
        " q quit   b buy   s sell   (live WS wiring: next step) ",
        Style::default().fg(Color::DarkGray),
    ));
    f.render_widget(Paragraph::new(line), area);
}

/// Orderbook ladder: asks (red) top-down to the spread, bids (green)
/// below. Depth bar width scales with qty.
fn draw_book(f: &mut Frame, area: Rect, app: &App) {
    let mut rows: Vec<Row> = Vec::new();
    for (px, qty) in &app.asks {
        rows.push(level_row(*px, *qty, Color::Red));
    }
    let spread = app.asks.last().map(|a| a.0).unwrap_or(0)
        - app.bids.first().map(|b| b.0).unwrap_or(0);
    rows.push(Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled(
            format!("spread {spread}"),
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
        Cell::from(Span::styled(
            px.to_string(),
            Style::default().fg(color),
        )),
        Cell::from(qty.to_string()),
        Cell::from(Span::styled(bar, Style::default().fg(color))),
    ])
}

fn draw_order_entry(f: &mut Frame, area: Rect, app: &App) {
    let (buy, sell) = match app.side {
        Side::Buy => (Modifier::REVERSED, Modifier::empty()),
        Side::Sell => (Modifier::empty(), Modifier::REVERSED),
    };
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
        Line::from("  price: 10001   (mock)"),
        Line::from("  qty:   10      (mock)"),
        Line::from("  tif:   GTC     (GTC/IOC/FOK)"),
        Line::from(""),
        Line::from(Span::styled(
            "  [enter] submit — disabled until WS lands",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(body).block(
        Block::default().borders(Borders::ALL).title(" order "),
    );
    f.render_widget(p, area);
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
            let pnl_color = if *upnl >= 0 { Color::Green } else { Color::Red };
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
    .header(Row::new(vec!["sym", "net", "entry", "upnl"]).style(
        Style::default().fg(Color::DarkGray),
    ))
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
    let p = Paragraph::new(lines).block(
        Block::default().borders(Borders::ALL).title(" trades "),
    );
    f.render_widget(p, area);
}
