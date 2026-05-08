// UsageMonitor TUI — adapted from Tokscale's ratatui architecture.
// MIT-licensed reference: https://github.com/junhoyeo/tokscale

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseEventKind},
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
const THEMES: usize = 5;

// ── App state (Tokscale app.rs pattern) ──

struct App {
    tab: Tab,
    scroll: usize,
    selected: usize,
    sort_col: u8,
    sort_desc: bool,
    theme: usize,
    last_refresh: Instant,
    needs_reload: bool,
    background_loading: bool,
    models: Vec<ModelRow>,
    sessions: Vec<SessRow>,
    daily: Vec<DailyRow>,
    heatmap: Vec<(String, i64)>,
    agents: Vec<AgentRow>,
    msg_count: i64,
    total_cost: f64,
    total_input: i64,
    total_output: i64,
    total_cache_read: i64,
}

#[derive(Clone, PartialEq)]
enum Tab { Overview, Models, Daily, Stats, Agents, Sessions }

impl Tab {
    fn next(&self) -> Self { match self { Tab::Overview=>Tab::Models,Tab::Models=>Tab::Daily,Tab::Daily=>Tab::Stats,Tab::Stats=>Tab::Agents,Tab::Agents=>Tab::Sessions,Tab::Sessions=>Tab::Overview } }
    fn name(&self) -> &str { match self { Tab::Overview=>"Overview",Tab::Models=>"Models",Tab::Daily=>"Daily",Tab::Stats=>"Stats",Tab::Agents=>"Agents",Tab::Sessions=>"Sessions" } }
    fn all() -> Vec<Tab> { vec![Tab::Overview,Tab::Models,Tab::Daily,Tab::Stats,Tab::Agents,Tab::Sessions] }
}

#[derive(Clone)]
struct ModelRow { model_id: String, input: i64, output: i64, cache_read: i64, cache_write: i64, sessions: usize, requests: usize, cost: f64 }
#[derive(Clone)]
struct SessRow { session_id: String, client: String, model_id: String, tokens: i64, cache_read: i64, messages: usize, cost: f64 }
#[derive(Clone)]
struct DailyRow { date: String, input: i64, output: i64, cache_read: i64, cache_write: i64, requests: usize, cost: f64 }
#[derive(Clone)]
struct AgentRow { name: String, client: String, tokens: i64, cost: f64, messages: i64 }

impl App {
    fn load(storage: &Storage, pricing: &PricingEngine) -> Self {
        let mut a = Self {
            tab: Tab::Overview, scroll: 0, selected: 0, sort_col: 0, sort_desc: true,
            theme: 0, last_refresh: Instant::now(), needs_reload: false, background_loading: false,
            models: vec![], sessions: vec![], daily: vec![], heatmap: vec![], agents: vec![],
            msg_count: 0, total_cost: 0.0, total_input: 0, total_output: 0, total_cache_read: 0,
        };
        a.reload(storage, pricing);
        a
    }

    fn reload(&mut self, storage: &Storage, pricing: &PricingEngine) {
        self.background_loading = true;
        let ms = storage.query_models().unwrap_or_default();
        let ss = storage.query_sessions().unwrap_or_default();
        self.msg_count = storage.messages_count().unwrap_or(0);

        self.models = ms.iter().map(|x| ModelRow {
            model_id: x.model_id.clone(), input: x.tokens.input, output: x.tokens.output,
            cache_read: x.tokens.cache_read, cache_write: x.tokens.cache_write,
            sessions: x.session_count, requests: x.request_count,
            cost: pricing.calculate_cost(&x.model_id, &x.tokens),
        }).collect();

        self.total_input = self.models.iter().map(|m| m.input).sum();
        self.total_output = self.models.iter().map(|m| m.output).sum();
        self.total_cache_read = self.models.iter().map(|m| m.cache_read).sum();
        self.total_cost = self.models.iter().map(|m| m.cost).sum();

        self.sessions = ss.iter().map(|x| SessRow {
            session_id: x.session_id.clone(), client: x.client.clone(),
            model_id: x.model_id.clone(), tokens: x.tokens.input + x.tokens.output,
            cache_read: x.tokens.cache_read, messages: x.message_count,
            cost: pricing.calculate_cost(&x.model_id, &x.tokens),
        }).collect();

        // Daily aggregation
        let conn = storage.lock();
        let mut stmt = conn.prepare(
            "SELECT date(timestamp/1000,'unixepoch','localtime') as d, SUM(input_tokens), SUM(output_tokens),
                    SUM(cache_read_tokens), SUM(cache_write_tokens), COUNT(*), SUM(cost_usd)
             FROM messages GROUP BY d ORDER BY d"
        ).unwrap();
        self.daily = stmt.query_map([], |r| Ok(DailyRow{
            date: r.get(0)?, input: r.get(1)?, output: r.get(2)?,
            cache_read: r.get(3)?, cache_write: r.get(4)?, requests: r.get::<_,i64>(5)? as usize, cost: r.get(6)?,
        })).unwrap().flatten().collect();

        self.heatmap = self.daily.iter().map(|d| (d.date.clone(), d.input + d.output)).collect();

        // Agents
        let mut agents: HashMap<String, (i64, f64, i64)> = HashMap::new();
        for s in &self.sessions {
            let key = format!("{}:{}", s.client, s.model_id);
            let e = agents.entry(key).or_default();
            e.0 += s.tokens; e.1 += s.cost; e.2 += s.messages as i64;
        }
        self.agents = agents.into_iter().map(|(k, (t, c, m))| {
            let parts: Vec<&str> = k.splitn(2, ':').collect();
            AgentRow { client: parts[0].into(), name: parts.get(1).unwrap_or(&"").to_string(), tokens: t, cost: c, messages: m }
        }).collect();
        self.agents.sort_by(|a,b| b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal));

        self.sort();
        self.background_loading = false;
    }

    fn sort(&mut self) {
        match self.sort_col {
            0 => self.models.sort_by(|a,b| if self.sort_desc { b.model_id.cmp(&a.model_id) } else { a.model_id.cmp(&b.model_id) }),
            1 => self.models.sort_by(|a,b| if self.sort_desc { b.sessions.cmp(&a.sessions) } else { a.sessions.cmp(&b.sessions) }),
            2 => self.models.sort_by(|a,b| if self.sort_desc { (b.input+b.output).cmp(&(a.input+a.output)) } else { (a.input+a.output).cmp(&(b.input+b.output)) }),
            3 => self.models.sort_by(|a,b| if self.sort_desc { b.cache_read.cmp(&a.cache_read) } else { a.cache_read.cmp(&b.cache_read) }),
            _ => self.models.sort_by(|a,b| if self.sort_desc { b.cost.partial_cmp(&a.cost).unwrap_or(std::cmp::Ordering::Equal) } else { a.cost.partial_cmp(&b.cost).unwrap_or(std::cmp::Ordering::Equal) }),
        }
    }
}

// ── Entry point ──

pub fn run(db_path: PathBuf, home: PathBuf) -> anyhow::Result<()> {
    let storage = Storage::open(&db_path)?;
    let cc_db = home.join(".cc-switch").join("cc-switch.db");
    let pricing = PricingEngine::new(if cc_db.exists() { cc_db.to_str() } else { None });
    let mut app = App::load(&storage, &pricing);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let res = event_loop(&mut terminal, &mut app, &storage, &pricing);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    res
}

fn event_loop<B: Backend>(t: &mut Terminal<B>, app: &mut App, storage: &Storage, pricing: &PricingEngine) -> anyhow::Result<()> {
    loop {
        t.draw(|f| render(f, app))?;
        if event::poll(Duration::from_millis(200))? {
            let ev = event::read()?;
            match ev {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    if handle_key(app, key.code) { return Ok(()); }
                }
                Event::Mouse(m) => handle_mouse(app, m),
                _ => {}
            }
        }
        // Auto-refresh
        if app.last_refresh.elapsed().as_secs() >= REFRESH_SECS && !app.background_loading {
            app.reload(storage, pricing);
            app.last_refresh = Instant::now();
        }
    }
}

fn handle_key(app: &mut App, key: KeyCode) -> bool {
    match key {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Tab => { let tabs = Tab::all(); let idx = tabs.iter().position(|t| *t == app.tab).unwrap_or(0); app.tab = tabs[(idx + 1) % tabs.len()].clone(); app.scroll = 0; app.selected = 0; }
        KeyCode::Char('1') => { app.tab = Tab::Overview; app.scroll = 0; app.selected = 0; }
        KeyCode::Char('2') => { app.tab = Tab::Models; app.scroll = 0; app.selected = 0; }
        KeyCode::Char('3') => { app.tab = Tab::Daily; app.scroll = 0; app.selected = 0; }
        KeyCode::Char('4') => { app.tab = Tab::Stats; app.scroll = 0; app.selected = 0; }
        KeyCode::Char('5') => { app.tab = Tab::Agents; app.scroll = 0; app.selected = 0; }
        KeyCode::Char('6') => { app.tab = Tab::Sessions; app.scroll = 0; app.selected = 0; }
        KeyCode::Char('t') => { app.theme = (app.theme + 1) % THEMES; }
        KeyCode::Char('s') => { app.sort_col = (app.sort_col + 1) % 5; app.sort_desc = !app.sort_desc; app.sort(); }
        KeyCode::Char('r') => { app.needs_reload = true; }
        KeyCode::Up | KeyCode::Char('k') => { if app.selected > 0 { app.selected -= 1; if app.selected < app.scroll { app.scroll = app.selected; } } }
        KeyCode::Down | KeyCode::Char('j') => { app.selected += 1; if app.selected >= app.scroll.saturating_add(20) { app.scroll = app.selected.saturating_sub(19); } }
        KeyCode::PageUp => { app.scroll = app.scroll.saturating_sub(10); app.selected = app.selected.saturating_sub(10); }
        KeyCode::PageDown => { app.scroll += 10; app.selected += 10; }
        KeyCode::Home => { app.scroll = 0; app.selected = 0; }
        KeyCode::End => { app.scroll = usize::MAX; app.selected = usize::MAX; }
        _ => {}
    }
    false
}

fn handle_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    match m.kind {
        MouseEventKind::ScrollDown => { app.scroll += 3; app.selected += 3; }
        MouseEventKind::ScrollUp => { app.scroll = app.scroll.saturating_sub(3); app.selected = app.selected.saturating_sub(3); }
        _ => {}
    }
}

// ── Theme colors (Tokscale themes.rs pattern) ──

fn theme(theme: usize) -> (Color, Color, Color, Color) {
    match theme {
        0 => (Color::Cyan, Color::Rgb(88,166,255), Color::Rgb(63,185,80), Color::Rgb(13,17,23)),     // blue
        1 => (Color::Magenta, Color::Rgb(188,140,255), Color::Rgb(210,153,29), Color::Rgb(22,16,30)), // purple
        2 => (Color::Green, Color::Rgb(63,185,80), Color::Rgb(88,166,255), Color::Rgb(13,23,17)),     // green
        3 => (Color::Yellow, Color::Rgb(210,153,29), Color::Rgb(88,166,255), Color::Rgb(23,20,13)),   // orange
        _ => (Color::Red, Color::Rgb(248,81,73), Color::Rgb(210,153,29), Color::Rgb(23,13,13)),       // red
    }
}

// ── Main render (Tokscale ui/mod.rs pattern) ──

fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Length(3), Constraint::Min(0), Constraint::Length(1)]).split(area);
    let (accent, accent2, accent3, bg) = theme(app.theme);

    // Header
    let chr = if app.total_input + app.total_cache_read > 0 { app.total_cache_read as f64 / (app.total_input + app.total_cache_read) as f64 } else { 0.0 };
    let hdr = Line::from(vec![
        Span::styled(" UsageMonitor ", Style::default().fg(Color::White).bold()),
        Span::styled(format!("│ {} ", fmt(app.total_input + app.total_output)), Style::default().fg(accent2)),
        Span::styled(format!("${:.2} ", app.total_cost), Style::default().fg(accent3)),
        Span::styled(format!("CHR {:.1}% ", chr * 100.0), Style::default().fg(Color::Yellow)),
        Span::styled(format!("{} msgs ", fmt(app.msg_count)), Style::default().fg(Color::Gray)),
        if app.background_loading { Span::styled("⏳", Style::default().fg(Color::Yellow)) } else { Span::styled("✓", Style::default().fg(Color::Green)) },
    ]);
    f.render_widget(Paragraph::new(hdr).bg(bg), chunks[0]);

    // Tabs
    let all_tabs = Tab::all();
    let tabs: Vec<&str> = all_tabs.iter().map(|t| t.name()).collect();
    let tab_idx = all_tabs.iter().position(|t| *t == app.tab).unwrap_or(0);
    let tab_widget = Tabs::new(tabs).select(tab_idx)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(Style::default().fg(Color::Black).bg(accent).bold());
    f.render_widget(tab_widget, chunks[1]);

    // Tab content
    match app.tab {
        Tab::Overview => tab_overview(f, chunks[2], app, accent, accent2, accent3),
        Tab::Models => tab_models(f, chunks[2], app, accent),
        Tab::Daily => tab_daily(f, chunks[2], app, accent),
        Tab::Stats => tab_stats(f, chunks[2], app, accent),
        Tab::Agents => tab_agents(f, chunks[2], app, accent),
        Tab::Sessions => tab_sessions(f, chunks[2], app, accent),
    }

    // Footer (Tokscale footer.rs pattern)
    let ft = Line::from(vec![
        Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::Red)),
        Span::styled(" quit  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" 1-6 ", Style::default().fg(Color::Black).bg(accent)),
        Span::styled(" tabs  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" s ", Style::default().fg(Color::Black).bg(accent2)),
        Span::styled(" sort  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" t ", Style::default().fg(Color::Black).bg(accent3)),
        Span::styled(" theme  ", Style::default().fg(Color::DarkGray)),
        Span::styled(" r ", Style::default().fg(Color::Black).bg(Color::Yellow)),
        Span::styled(" refresh  ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" {}s auto ", REFRESH_SECS), Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(ft).bg(bg), chunks[3]);
}

// ── Tab renderers (Tokscale ui/*.rs patterns) ──

fn tab_overview(f: &mut Frame, area: Rect, app: &App, accent: Color, accent2: Color, accent3: Color) {
    let rows = Layout::vertical([Constraint::Length(8), Constraint::Length(4), Constraint::Min(0)]).split(area);

    // KPI cards
    let cards = Layout::horizontal([Constraint::Ratio(1,4);4]).split(rows[0]);
    let total_used = app.total_input + app.total_output + app.models.iter().map(|m| m.cache_write).sum::<i64>();
    let chr = if app.total_input + app.total_cache_read > 0 { app.total_cache_read as f64 / (app.total_input + app.total_cache_read) as f64 } else { 0.0 };

    let vals = [fmt(total_used), format!("${:.2}", app.total_cost), format!("{:.1}%", chr*100.0), fmt(app.msg_count)];
    let subs = ["total tokens", "estimated", "cache hit rate", &format!("{} models", app.models.len())];
    let colors = [Color::White, accent3, Color::Yellow, accent2];
    for i in 0..4 {
        let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(48,54,61)));
        let p = Paragraph::new(vec![
            Line::from(Span::styled(subs[i], Style::default().fg(Color::Gray))),
            Line::from(Span::styled(vals[i].clone(), Style::default().fg(colors[i]).bold())),
        ]).block(block);
        f.render_widget(p, cards[i]);
    }

    // Token breakdown bar (stacked: input | output | cache)
    let max_t = (app.total_input + app.total_output + app.total_cache_read).max(1) as f64;
    let bars = Layout::horizontal([
        Constraint::Ratio((app.total_input as f64 / max_t * 100.0) as u32, 100),
        Constraint::Ratio((app.total_output as f64 / max_t * 100.0) as u32, 100),
        Constraint::Ratio((app.total_cache_read as f64 / max_t * 100.0) as u32, 100),
    ]).split(rows[1]);
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(accent2).bg(accent2)).ratio(1.0).label("Input"), bars[0]);
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(Color::Magenta).bg(Color::Magenta)).ratio(1.0).label("Output"), bars[1]);
    f.render_widget(Gauge::default().gauge_style(Style::default().fg(accent3).bg(accent3)).ratio(1.0).label("Cache Read"), bars[2]);

    // Top models
    let mrows: Vec<Row> = app.models.iter().take(14).map(|m| Row::new(vec![
        Cell::from(m.model_id.as_str()),
        Cell::from(fmt(m.input + m.output)).style(Style::default().fg(Color::White)),
        Cell::from(fmt(m.cache_read)).style(Style::default().fg(accent3)),
        Cell::from(format!("${:.2}", m.cost)).style(Style::default().fg(accent3)),
    ])).collect();
    let w = [Constraint::Percentage(40), Constraint::Percentage(22), Constraint::Percentage(22), Constraint::Percentage(16)];
    f.render_widget(
        Table::new(mrows, w).header(Row::new(vec!["Model","Tokens","Cache","Cost"]).style(Style::default().fg(Color::DarkGray)))
            .block(Block::default().borders(Borders::ALL).title(" Top Models ").border_style(Style::default().fg(Color::Rgb(48,54,61)))),
        rows[2],
    );
}

fn tab_models(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let cols = ["Model","Sessions","Tokens","Cache","Cost"];
    let hdr = Row::new(cols.iter().enumerate().map(|(i,c)| Cell::from(*c).style(if i as u8 == app.sort_col { Style::default().fg(accent).bold() } else { Style::default() }))).style(Style::default().fg(Color::Cyan));
    let rows: Vec<Row> = app.models.iter().skip(app.scroll).take(20).map(|m| Row::new(vec![
        Cell::from(m.model_id.as_str()), Cell::from(format!("{}", m.sessions)),
        Cell::from(fmt(m.input + m.output)), Cell::from(fmt(m.cache_read)),
        Cell::from(format!("${:.2}", m.cost)),
    ])).collect();
    let w = [Constraint::Percentage(34),Constraint::Percentage(12),Constraint::Percentage(20),Constraint::Percentage(18),Constraint::Percentage(16)];
    f.render_widget(Table::new(rows,w).header(hdr).block(Block::default().borders(Borders::ALL).title(format!(" Models ({}) s:sort ",app.models.len()))), area);
}

fn tab_daily(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let rows: Vec<Row> = app.daily.iter().rev().skip(app.scroll).take(22).map(|d| Row::new(vec![
        Cell::from(d.date.as_str()).style(if d.date == chrono::Local::now().format("%Y-%m-%d").to_string() { Style::default().fg(Color::Yellow).bold() } else { Style::default() }),
        Cell::from(fmt(d.input)), Cell::from(fmt(d.output)), Cell::from(fmt(d.cache_read)),
        Cell::from(fmt(d.cache_write)), Cell::from(format!("{}", d.requests)),
        Cell::from(format!("${:.4}", d.cost)),
    ])).collect();
    let w = [Constraint::Percentage(18),Constraint::Percentage(16),Constraint::Percentage(16),Constraint::Percentage(16),Constraint::Percentage(16),Constraint::Percentage(10),Constraint::Percentage(8)];
    f.render_widget(Table::new(rows,w).header(Row::new(vec!["Date","Input","Output","Cache R","Cache W","Reqs","Cost"]).style(Style::default().fg(accent)))
        .block(Block::default().borders(Borders::ALL).title(format!(" Daily ({}) ",app.daily.len()))), area);
}

fn tab_stats(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let chunks = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(area);
    let cards = Layout::horizontal([Constraint::Ratio(1,4);4]).split(chunks[0]);

    // Streaks
    let mut dates: Vec<String> = app.heatmap.iter().map(|(d,_)| d.clone()).collect();
    dates.sort(); dates.dedup();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let streak = calc_streak(&dates, &today);
    let longest = calc_longest(&dates);
    let active = dates.len();
    let total_days = if let (Some(f),Some(l)) = (dates.first(),dates.last()) {
        (chrono::NaiveDate::parse_from_str(l,"%Y-%m-%d").unwrap() - chrono::NaiveDate::parse_from_str(f,"%Y-%m-%d").unwrap()).num_days() as usize + 1
    } else {1};

    let svals = [format!("{} days",streak),format!("{} days",longest),format!("{}/{}",active,total_days),format!("${:.2}",app.total_cost)];
    let slabs = ["Current Streak","Longest Streak","Active Days","Total Cost"];
    let scols = [accent,Color::Yellow,Color::White,Color::LightGreen];
    for i in 0..4 {
        let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(48,54,61)));
        f.render_widget(Paragraph::new(vec![
            Line::from(Span::styled(slabs[i],Style::default().fg(Color::Gray))),
            Line::from(Span::styled(svals[i].clone(),Style::default().fg(scols[i]).bold())),
        ]).block(block), cards[i]);
    }

    // Text heatmap
    if !app.heatmap.is_empty() {
        let by_date: HashMap<String,i64> = app.heatmap.iter().cloned().collect();
        let max_v = by_date.values().max().copied().unwrap_or(1).max(1);
        let cols = 26; let end = chrono::Local::now().date_naive();
        let start = end - chrono::Duration::days(cols * 7 - 1);
        let mut lines = vec![];
        for row in 0..7 {
            let mut spans = vec![Span::styled(format!("{:2} ", if row%2==0 {""} else {""}), Style::default().fg(Color::DarkGray))];
            for col in 0..cols {
                let ds = (start + chrono::Duration::days((col*7+row) as i64)).format("%Y-%m-%d").to_string();
                let v = by_date.get(&ds).copied().unwrap_or(0);
                let (ch, c) = if v == 0 { ('·', Color::Rgb(22,27,34)) }
                else if v < max_v/3 { ('░', Color::Rgb(14,68,41)) }
                else if v < max_v*2/3 { ('▒', Color::Rgb(0,109,50)) }
                else { ('▓', Color::Rgb(57,211,83)) };
                spans.push(Span::styled(format!("{} ",ch), Style::default().fg(c)));
            }
            lines.push(Line::from(spans));
        }
        f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Contribution Heatmap ")), chunks[1]);
    }
}

fn tab_agents(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let rows: Vec<Row> = app.agents.iter().skip(app.scroll).take(20).map(|a| Row::new(vec![
        Cell::from(a.client.as_str()), Cell::from(a.name.as_str()),
        Cell::from(fmt(a.tokens)), Cell::from(format!("${:.2}",a.cost)),
        Cell::from(fmt(a.messages)),
    ])).collect();
    let w = [Constraint::Percentage(20),Constraint::Percentage(30),Constraint::Percentage(20),Constraint::Percentage(15),Constraint::Percentage(15)];
    f.render_widget(Table::new(rows,w).header(Row::new(vec!["Client","Model","Tokens","Cost","Msgs"]).style(Style::default().fg(accent)))
        .block(Block::default().borders(Borders::ALL).title(format!(" Agents ({}) ",app.agents.len()))), area);
}

fn tab_sessions(f: &mut Frame, area: Rect, app: &App, accent: Color) {
    let rows: Vec<Row> = app.sessions.iter().skip(app.scroll).take(20).map(|s| Row::new(vec![
        Cell::from(format!("{}…",&s.session_id[..8.min(s.session_id.len())])).style(Style::default().fg(accent)),
        Cell::from(s.client.as_str()), Cell::from(s.model_id.as_str()),
        Cell::from(fmt(s.tokens)), Cell::from(fmt(s.messages as i64)),
        Cell::from(format!("${:.2}",s.cost)),
    ])).collect();
    let w = [Constraint::Percentage(20),Constraint::Percentage(12),Constraint::Percentage(28),Constraint::Percentage(18),Constraint::Percentage(10),Constraint::Percentage(12)];
    f.render_widget(Table::new(rows,w).header(Row::new(vec!["Session","Client","Model","Tokens","Msgs","Cost"]).style(Style::default().fg(accent)))
        .block(Block::default().borders(Borders::ALL).title(format!(" Sessions ({}) ",app.sessions.len()))), area);
}

// ── Helpers ──

fn calc_streak(dates: &[String], today: &str) -> usize {
    let mut streak = 0;
    let mut d = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d").unwrap();
    loop {
        let ds = d.format("%Y-%m-%d").to_string();
        if dates.contains(&ds) { streak += 1; d -= chrono::Duration::days(1); } else { break; }
    }
    streak
}

fn calc_longest(dates: &[String]) -> usize {
    let mut sorted: Vec<chrono::NaiveDate> = dates.iter().filter_map(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).collect();
    sorted.sort(); sorted.dedup();
    let mut longest = 0; let mut cur = 0; let mut prev: Option<chrono::NaiveDate> = None;
    for d in sorted {
        if let Some(p) = prev {
            if d == p + chrono::Duration::days(1) { cur += 1; } else { cur = 1; }
        } else { cur = 1; }
        longest = longest.max(cur); prev = Some(d);
    }
    longest
}

fn fmt(n: i64) -> String {
    if n >= 1_000_000_000 { format!("{:.1}B", n as f64 / 1e9) }
    else if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1e6) }
    else if n >= 1_000 { format!("{:.0}K", n as f64 / 1e3) }
    else { n.to_string() }
}
