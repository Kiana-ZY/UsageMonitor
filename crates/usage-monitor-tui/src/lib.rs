use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table, Tabs},
    Frame, Terminal,
};
use usage_monitor_pricing::PricingEngine;
use usage_monitor_storage::Storage;

const REFRESH_SECS: u64 = 15;
const THEME_COUNT: usize = 5;

struct App {
    tab: usize,
    scroll: usize,
    selected: usize,
    sort_col: u8,
    sort_desc: bool,
    theme: usize,
    last_refresh: Instant,
    models: Vec<ModelRow>,
    sessions: Vec<SessRow>,
    daily: Vec<DailyRow>,
    heatmap: Vec<(String, i64)>,
    msg_count: i64,
    total_cost: f64,
}

#[derive(Clone)]
struct ModelRow { model_id: String, input: i64, output: i64, cache_read: i64, cache_write: i64, sessions: usize, cost: f64 }
#[derive(Clone)]
struct SessRow { session_id: String, client: String, model_id: String, tokens: i64, cache_read: i64, messages: usize, cost: f64 }
#[derive(Clone)]
struct DailyRow { date: String, input: i64, output: i64, cache_read: i64, requests: usize }

impl App {
    fn new(storage: &Storage, pricing: &PricingEngine) -> Self {
        let mut a = Self {
            tab: 0, scroll: 0, selected: 0, sort_col: 0, sort_desc: true, theme: 0,
            last_refresh: Instant::now(),
            models: vec![], sessions: vec![], daily: vec![], heatmap: vec![],
            msg_count: 0, total_cost: 0.0,
        };
        a.reload(storage, pricing);
        a
    }

    fn reload(&mut self, storage: &Storage, pricing: &PricingEngine) {
        let m = storage.query_models().unwrap_or_default();
        let s = storage.query_sessions().unwrap_or_default();
        self.msg_count = storage.messages_count().unwrap_or(0);
        self.models = m.iter().map(|x| ModelRow {
            model_id: x.model_id.clone(), input: x.tokens.input, output: x.tokens.output,
            cache_read: x.tokens.cache_read, cache_write: x.tokens.cache_write,
            sessions: x.session_count, cost: pricing.calculate_cost(&x.model_id, &x.tokens),
        }).collect();
        self.sessions = s.iter().map(|x| SessRow {
            session_id: x.session_id.clone(), client: x.client.clone(),
            model_id: x.model_id.clone(), tokens: x.tokens.input + x.tokens.output,
            cache_read: x.tokens.cache_read, messages: x.message_count,
            cost: pricing.calculate_cost(&x.model_id, &x.tokens),
        }).collect();
        self.total_cost = self.models.iter().map(|m| m.cost).sum();

        // Daily from messages
        let conn = storage.lock();
        let mut stmt = conn.prepare(
            "SELECT date(timestamp/1000,'unixepoch','localtime') as d, SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens), COUNT(*) FROM messages GROUP BY d ORDER BY d"
        ).unwrap();
        self.daily = stmt.query_map([], |r| Ok(DailyRow{
            date: r.get(0)?, input: r.get(1)?, output: r.get(2)?, cache_read: r.get(3)?, requests: r.get::<_,i64>(4)? as usize,
        })).unwrap().flatten().collect();
        self.heatmap = self.daily.iter().map(|d| (d.date.clone(), d.input + d.output)).collect();
        self.sort_models();
    }

    fn sort_models(&mut self) {
        match self.sort_col {
            0 => self.models.sort_by(|a,b| if self.sort_desc { b.model_id.cmp(&a.model_id) } else { a.model_id.cmp(&b.model_id) }),
            1 => self.models.sort_by(|a,b| if self.sort_desc { b.sessions.cmp(&a.sessions) } else { a.sessions.cmp(&b.sessions) }),
            2 => self.models.sort_by(|a,b| if self.sort_desc { (b.input+b.output).cmp(&(a.input+a.output)) } else { (a.input+a.output).cmp(&(b.input+b.output)) }),
            3 => self.models.sort_by(|a,b| if self.sort_desc { b.cache_read.cmp(&a.cache_read) } else { a.cache_read.cmp(&b.cache_read) }),
            4 => self.models.sort_by(|a,b| if self.sort_desc { b.cost.partial_cmp(&a.cost).unwrap() } else { a.cost.partial_cmp(&b.cost).unwrap() }),
            _ => {}
        }
    }
}

pub fn run(db_path: PathBuf, home: PathBuf) -> anyhow::Result<()> {
    let storage = Storage::open(&db_path)?;
    let cc_db = home.join(".cc-switch").join("cc-switch.db");
    let pricing = PricingEngine::new(if cc_db.exists() { cc_db.to_str() } else { None });
    let mut app = App::new(&storage, &pricing);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let res = run_loop(&mut terminal, &mut app, &storage, &pricing);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    res
}

fn run_loop<B: Backend>(t: &mut Terminal<B>, app: &mut App, storage: &Storage, pricing: &PricingEngine) -> anyhow::Result<()> {
    loop {
        t.draw(|f| ui(f, app))?;
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release { continue; }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('1') => { app.tab=0; app.scroll=0; app.selected=0; }
                    KeyCode::Char('2') => { app.tab=1; app.scroll=0; app.selected=0; }
                    KeyCode::Char('3') => { app.tab=2; app.scroll=0; app.selected=0; }
                    KeyCode::Char('4') => { app.tab=3; app.scroll=0; app.selected=0; }
                    KeyCode::Char('5') => { app.tab=4; app.scroll=0; app.selected=0; }
                    KeyCode::Tab => { app.tab = (app.tab + 1) % 5; app.scroll=0; app.selected=0; }
                    KeyCode::Char('t') => { app.theme = (app.theme + 1) % THEME_COUNT; }
                    KeyCode::Up|KeyCode::Char('k') => { if app.selected>0 {app.selected-=1; if app.selected<app.scroll {app.scroll=app.selected;}} }
                    KeyCode::Down|KeyCode::Char('j') => { app.selected+=1; if app.selected>=app.scroll.saturating_add(20) {app.scroll=app.selected.saturating_sub(19);} }
                    KeyCode::PageUp => { app.scroll=app.scroll.saturating_sub(10); app.selected=app.selected.saturating_sub(10); }
                    KeyCode::PageDown => { app.scroll+=10; app.selected+=10; }
                    KeyCode::Home => { app.scroll=0; app.selected=0; }
                    KeyCode::End => { app.scroll=usize::MAX; app.selected=usize::MAX; }
                    KeyCode::Char('s') if app.tab == 1 => { app.sort_col = (app.sort_col+1)%5; app.sort_desc = !app.sort_desc; app.sort_models(); }
                    KeyCode::Char('r') => { app.reload(storage, pricing); app.last_refresh=Instant::now(); }
                    _ => {}
                }
            }
        }
        if app.last_refresh.elapsed().as_secs() >= REFRESH_SECS {
            app.reload(storage, pricing);
            app.last_refresh = Instant::now();
        }
    }
}

fn theme_colors(theme: usize) -> (Color, Color, Color) {
    match theme {
        0 => (Color::Cyan, Color::Rgb(88,166,255), Color::Rgb(63,185,80)),   // blue
        1 => (Color::Magenta, Color::Rgb(188,140,255), Color::Rgb(210,153,29)), // purple
        2 => (Color::Green, Color::Rgb(63,185,80), Color::Rgb(88,166,255)),  // green
        3 => (Color::Yellow, Color::Rgb(210,153,29), Color::Rgb(88,166,255)), // orange
        _ => (Color::Red, Color::Rgb(248,81,73), Color::Rgb(88,166,255)),     // red
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)]).split(area);
    let (accent, accent2, accent3) = theme_colors(app.theme);

    // Header bar
    let mut total_in: i64 = 0; let mut total_out: i64 = 0; let mut total_cr: i64 = 0;
    for m in &app.models { total_in += m.input; total_out += m.output; total_cr += m.cache_read; }
    let chr = if total_in + total_cr > 0 { total_cr as f64 / (total_in + total_cr) as f64 } else { 0.0 };

    let hdr = Line::from(vec![
        Span::styled(format!(" UsageMonitor "), Style::default().fg(Color::White).bold()),
        Span::styled(format!("│ {} ", fmt(total_in+total_out)), Style::default().fg(accent2)),
        Span::styled(format!("${:.2} ", app.total_cost), Style::default().fg(accent3)),
        Span::styled(format!("CHR {:.1}% ", chr*100.0), Style::default().fg(Color::Yellow)),
        Span::styled(format!("{} msgs ", fmt(app.msg_count)), Style::default().fg(Color::Gray)),
    ]);
    f.render_widget(Paragraph::new(hdr).bg(Color::Rgb(13,17,23)), chunks[0]);

    // Tabs
    let tab_labels = vec![" Overview ", " Models ", " Daily ", " Stats ", " Sessions "];
    let tabs = Tabs::new(tab_labels).select(app.tab)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(Style::default().fg(Color::Black).bg(accent).bold());
    f.render_widget(tabs, chunks[1]);

    // Tab content
    match app.tab {
        0 => tab_overview(f, chunks[2], app, accent, accent2, accent3),
        1 => tab_models(f, chunks[2], app, accent),
        2 => tab_daily(f, chunks[2], app, accent),
        3 => tab_stats(f, chunks[2], app, accent),
        4 => tab_sessions(f, chunks[2], app, accent),
        _ => {}
    }

    // Footer
    let ft = Line::from(vec![
        Span::styled(" q:quit ", Style::default().fg(Color::DarkGray)),
        Span::styled("1-5:tab ", Style::default().fg(Color::DarkGray)),
        Span::styled("s:sort ", Style::default().fg(Color::DarkGray)),
        Span::styled("t:theme ", Style::default().fg(Color::DarkGray)),
        Span::styled("r:refresh ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{}s ", REFRESH_SECS), Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(ft).bg(Color::Rgb(13,17,23)), chunks[3]);
}

fn tab_overview(f: &mut Frame, area: Rect, app: &App, accent: Color, accent2: Color, accent3: Color) {
    let rows = Layout::vertical([Constraint::Length(8), Constraint::Length(4), Constraint::Min(0)]).split(area);

    // Cards
    let cards = Layout::horizontal([Constraint::Ratio(1,4);4]).split(rows[0]);
    let bk = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(48,54,61)));
    let total_used: i64 = app.models.iter().map(|m| m.input + m.output + m.cache_write).sum();
    let total_in: i64 = app.models.iter().map(|m| m.input).sum();
    let total_cr: i64 = app.models.iter().map(|m| m.cache_read).sum();
    let chr = if total_in + total_cr > 0 { total_cr as f64 / (total_in + total_cr) as f64 } else { 0.0 };

    let t1 = fmt(total_used); let t2 = format!("${:.2}", app.total_cost);
    let t3 = format!("{:.1}%", chr*100.0); let t4 = fmt(app.msg_count); let t4s = format!("{} models", app.models.len());
    let card = |t: &str, v: &str, s: &str, c: Color| {
        Paragraph::new(vec![
            Line::from(Span::styled(t.to_string(), Style::default().fg(Color::Gray))),
            Line::from(Span::styled(v.to_string(), Style::default().fg(c).bold())),
            Line::from(Span::styled(s.to_string(), Style::default().fg(Color::DarkGray))),
        ]).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(48,54,61))))
    };
    f.render_widget(card("Tokens", &t1, "total", Color::White), cards[0]);
    f.render_widget(card("Cost", &t2, "estimated", accent3), cards[1]);
    f.render_widget(card("Cache Hit", &t3, "read/total", Color::Yellow), cards[2]);
    f.render_widget(card("Messages", &t4, &t4s, accent2), cards[3]);

    // Token breakdown bar
    let max_t = (total_in + app.models.iter().map(|m|m.output).sum::<i64>() + total_cr).max(1) as f64;
    let out: i64 = app.models.iter().map(|m|m.output).sum();
    let bar = Layout::horizontal([
        Constraint::Ratio((total_in as f64 / max_t * 100.0) as u32, 100),
        Constraint::Ratio((out as f64 / max_t * 100.0) as u32, 100),
        Constraint::Ratio((total_cr as f64 / max_t * 100.0) as u32, 100),
    ]).split(rows[1]);
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(accent2).bg(accent2)).ratio(1.0).label("Input"), bar[0]);
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(Color::Magenta).bg(Color::Magenta)).ratio(1.0).label("Output"), bar[1]);
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(accent3).bg(accent3)).ratio(1.0).label("Cache"), bar[2]);

    // Top models
    let model_rows: Vec<Row> = app.models.iter().take(12).map(|m| {
        Row::new(vec![
            Cell::from(m.model_id.as_str()),
            Cell::from(fmt(m.input+m.output)).style(Style::default().fg(Color::White)),
            Cell::from(fmt(m.cache_read)).style(Style::default().fg(accent3)),
            Cell::from(format!("${:.2}", m.cost)).style(Style::default().fg(accent3)),
        ])
    }).collect();
    let w = [Constraint::Percentage(40), Constraint::Percentage(22), Constraint::Percentage(22), Constraint::Percentage(16)];
    let tbl = Table::new(model_rows, w)
        .header(Row::new(vec!["Model","Tokens","Cache","Cost"]).style(Style::default().fg(Color::DarkGray)))
        .block(Block::default().borders(Borders::ALL).title("Top Models").border_style(Style::default().fg(Color::Rgb(48,54,61))));
    f.render_widget(tbl, rows[2]);
}

fn tab_models(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let headers = vec!["Model","Sessions","Input","Output","Cache","Cost"];
    let hdr = Row::new(headers.iter().map(|h| Cell::from(*h).style(if app.sort_col == headers.iter().position(|x|*x==*h).unwrap_or(99) as u8 { Style::default().fg(accent).bold() } else { Style::default() })))
        .style(Style::default().fg(Color::Cyan));
    let rows: Vec<Row> = app.models.iter().skip(app.scroll).take(20).map(|m|
        Row::new(vec![
            Cell::from(m.model_id.as_str()),
            Cell::from(format!("{}",m.sessions)), Cell::from(fmt(m.input)),
            Cell::from(fmt(m.output)), Cell::from(fmt(m.cache_read)),
            Cell::from(format!("${:.2}",m.cost)),
        ])
    ).collect();
    let w = [Constraint::Percentage(32),Constraint::Percentage(10),Constraint::Percentage(16),Constraint::Percentage(16),Constraint::Percentage(16),Constraint::Percentage(10)];
    let tbl = Table::new(rows, w).header(hdr)
        .block(Block::default().borders(Borders::ALL).title(format!("Models ({}) s:sort", app.models.len())));
    f.render_widget(tbl, area);
}

fn tab_daily(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let rows: Vec<Row> = app.daily.iter().rev().skip(app.scroll).take(25).map(|d|
        Row::new(vec![
            Cell::from(d.date.as_str()), Cell::from(fmt(d.input)),
            Cell::from(fmt(d.output)), Cell::from(fmt(d.cache_read)),
            Cell::from(format!("{}",d.requests)),
        ])
    ).collect();
    let w = [Constraint::Percentage(25),Constraint::Percentage(25),Constraint::Percentage(25),Constraint::Percentage(25),Constraint::Percentage(0)]; // fixed
    let w2 = [Constraint::Percentage(20),Constraint::Percentage(20),Constraint::Percentage(20),Constraint::Percentage(20),Constraint::Percentage(20)];
    let tbl = Table::new(rows, w2).header(Row::new(vec!["Date","Input","Output","Cache","Reqs"]).style(Style::default().fg(accent)))
        .block(Block::default().borders(Borders::ALL).title(format!("Daily ({})", app.daily.len())));
    f.render_widget(tbl, area);
}

fn tab_stats(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let chunks = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(area);

    // Streak stats
    let streak_cards = Layout::horizontal([Constraint::Ratio(1,4);4]).split(chunks[0]);
    let mut sorted_dates: Vec<_> = app.heatmap.iter().map(|(d,_)| d.clone()).collect();
    sorted_dates.sort(); sorted_dates.dedup();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let current_streak = calc_streak(&sorted_dates, &today);
    let longest_streak = calc_longest(&sorted_dates);
    let active_days = sorted_dates.len();
    let total_days = if let (Some(first), Some(last)) = (sorted_dates.first(), sorted_dates.last()) {
        let f = chrono::NaiveDate::parse_from_str(first, "%Y-%m-%d").unwrap();
        let l = chrono::NaiveDate::parse_from_str(last, "%Y-%m-%d").unwrap();
        (l - f).num_days() as usize + 1
    } else { 1 };

    let s1 = format!("{} days", current_streak); let s2 = format!("{} days", longest_streak);
    let s3 = format!("{} / {}", active_days, total_days); let s4 = format!("${:.2}", app.total_cost);
    let sc = |t: &str, v: &str, c: Color| {
        Paragraph::new(vec![
            Line::from(Span::styled(t.to_string(), Style::default().fg(Color::Gray))),
            Line::from(Span::styled(v.to_string(), Style::default().fg(c).bold())),
        ]).block(Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(48,54,61))))
    };
    f.render_widget(sc("Current Streak", &s1, accent), streak_cards[0]);
    f.render_widget(sc("Longest Streak", &s2, Color::Yellow), streak_cards[1]);
    f.render_widget(sc("Active Days", &s3, Color::White), streak_cards[2]);
    f.render_widget(sc("Total Cost", &s4, Color::LightGreen), streak_cards[3]);

    // Heatmap (simplified text grid)
    if !app.heatmap.is_empty() {
        let mut by_date: HashMap<String, i64> = app.heatmap.iter().cloned().collect();
        let max_v = by_date.values().max().copied().unwrap_or(1).max(1);
        let cols = 26;
        let end = chrono::Local::now().date_naive();
        let start = end - chrono::Duration::days(cols * 7 - 1);

        let mut lines: Vec<Line> = vec![];
        for row in 0..7 {
            let mut spans = vec![];
            for col in 0..cols {
                let d = start + chrono::Duration::days((col * 7 + row) as i64);
                let ds = d.format("%Y-%m-%d").to_string();
                let v = by_date.get(&ds).copied().unwrap_or(0);
                let ch = if v == 0 { '·' } else if v < max_v / 3 { '░' } else if v < max_v * 2 / 3 { '▒' } else { '▓' };
                let c = if v == 0 { Color::Rgb(22,27,34) } else if v < max_v / 3 { Color::Rgb(14,68,41) } else if v < max_v * 2 / 3 { Color::Rgb(0,109,50) } else { Color::Rgb(57,211,83) };
                spans.push(Span::styled(format!("{} ", ch), Style::default().fg(c)));
            }
            lines.push(Line::from(spans));
        }
        let p = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Contribution Heatmap"));
        f.render_widget(p, chunks[1]);
    }
}

fn tab_sessions(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let rows: Vec<Row> = app.sessions.iter().skip(app.scroll).take(20).map(|s|
        Row::new(vec![
            Cell::from(format!("{}…", &s.session_id[..8.min(s.session_id.len())])).style(Style::default().fg(accent)),
            Cell::from(s.client.as_str()), Cell::from(s.model_id.as_str()),
            Cell::from(fmt(s.tokens)), Cell::from(fmt(s.messages as i64)),
            Cell::from(format!("${:.2}",s.cost)),
        ])
    ).collect();
    let w = [Constraint::Percentage(20),Constraint::Percentage(12),Constraint::Percentage(28),Constraint::Percentage(18),Constraint::Percentage(10),Constraint::Percentage(12)];
    let tbl = Table::new(rows, w).header(Row::new(vec!["Session","Client","Model","Tokens","Msgs","Cost"]).style(Style::default().fg(accent)))
        .block(Block::default().borders(Borders::ALL).title(format!("Sessions ({})", app.sessions.len())));
    f.render_widget(tbl, area);
}

fn calc_streak(dates: &[String], today: &str) -> usize {
    let mut set: Vec<_> = dates.iter().collect();
    set.sort(); set.dedup();
    let mut streak = 0;
    let mut d = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d").unwrap();
    loop {
        if set.contains(&&d.format("%Y-%m-%d").to_string()) { streak += 1; } else { break; }
        d = d - chrono::Duration::days(1);
    }
    streak
}

fn calc_longest(dates: &[String]) -> usize {
    let mut set: Vec<_> = dates.iter().collect();
    set.sort(); set.dedup();
    let mut longest = 0; let mut cur = 0;
    let mut prev: Option<chrono::NaiveDate> = None;
    for ds in &set {
        let d = chrono::NaiveDate::parse_from_str(ds, "%Y-%m-%d").unwrap();
        if let Some(p) = prev {
            if d == p + chrono::Duration::days(1) { cur += 1; } else { cur = 1; }
        } else { cur = 1; }
        longest = longest.max(cur);
        prev = Some(d);
    }
    longest
}

fn fmt(n: i64) -> String {
    if n >= 1_000_000_000 { format!("{:.1}B", n as f64 / 1e9) }
    else if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1e6) }
    else if n >= 1_000 { format!("{:.0}K", n as f64 / 1e3) }
    else { n.to_string() }
}
