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

const REFRESH_SECS: u64 = 10;

pub fn run(db_path: PathBuf, home: PathBuf) -> anyhow::Result<()> {
    let storage = Storage::open(&db_path)?;
    let cc_db = home.join(".cc-switch").join("cc-switch.db");
    let pricing = PricingEngine::new(if cc_db.exists() { cc_db.to_str() } else { None });

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let res = app_loop(&mut terminal, &storage, &pricing);
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    res
}

struct ModelData {
    model_id: String,
    input: i64, output: i64, cache_read: i64, cache_write: i64,
    sessions: usize, requests: usize, cost: f64,
}

struct SessionData {
    session_id: String, client: String, model_id: String,
    input: i64, output: i64, cache_read: i64, messages: usize, cost: f64,
}

fn load_data(storage: &Storage, pricing: &PricingEngine) -> (Vec<ModelData>, Vec<SessionData>, i64, f64) {
    let models = storage.query_models().unwrap_or_default();
    let sessions = storage.query_sessions().unwrap_or_default();
    let count = storage.messages_count().unwrap_or(0);

    let md: Vec<ModelData> = models.iter().map(|m| ModelData {
        model_id: m.model_id.clone(),
        input: m.tokens.input, output: m.tokens.output,
        cache_read: m.tokens.cache_read, cache_write: m.tokens.cache_write,
        sessions: m.session_count, requests: m.request_count,
        cost: pricing.calculate_cost(&m.model_id, &m.tokens),
    }).collect();

    let sd: Vec<SessionData> = sessions.iter().map(|s| SessionData {
        session_id: s.session_id.clone(), client: s.client.clone(),
        model_id: s.model_id.clone(),
        input: s.tokens.input, output: s.tokens.output,
        cache_read: s.tokens.cache_read, messages: s.message_count,
        cost: pricing.calculate_cost(&s.model_id, &s.tokens),
    }).collect();

    let total_cost = md.iter().map(|m| m.cost).sum();
    (md, sd, count, total_cost)
}

fn app_loop<B: Backend>(terminal: &mut Terminal<B>, storage: &Storage, pricing: &PricingEngine) -> anyhow::Result<()> {
    let mut tab: usize = 0;
    let mut scroll: usize = 0;
    let mut selected: usize = 0;
    let mut last_refresh = Instant::now();
    let (mut models, mut sessions, mut msg_count, mut total_cost) = load_data(storage, pricing);

    loop {
        terminal.draw(|f| {
            ui(f, tab, &models, &sessions, msg_count, total_cost, scroll, selected);
        })?;

        // Non-blocking event poll
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Release { continue; }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('1') => { tab = 0; scroll = 0; selected = 0; }
                    KeyCode::Char('2') => { tab = 1; scroll = 0; selected = 0; }
                    KeyCode::Char('3') => { tab = 2; scroll = 0; selected = 0; }
                    KeyCode::Tab => { tab = (tab + 1) % 3; scroll = 0; selected = 0; }
                    KeyCode::Up | KeyCode::Char('k') => { if selected > 0 { selected -= 1; if selected < scroll { scroll = selected; } } }
                    KeyCode::Down | KeyCode::Char('j') => { selected += 1; if selected >= scroll + 20 { scroll = selected - 19; } }
                    KeyCode::PageUp => { scroll = scroll.saturating_sub(10); }
                    KeyCode::PageDown => { scroll += 10; }
                    KeyCode::Home => { selected = 0; scroll = 0; }
                    KeyCode::End => { selected = usize::MAX; scroll = usize::MAX; }
                    KeyCode::Char('r') => {
                        let (m, s, c, cost) = load_data(storage, pricing);
                        models = m; sessions = s; msg_count = c; total_cost = cost;
                        last_refresh = Instant::now();
                    }
                    _ => {}
                }
            }
        }

        // Auto-refresh
        if last_refresh.elapsed().as_secs() >= REFRESH_SECS {
            let (m, s, c, cost) = load_data(storage, pricing);
            models = m; sessions = s; msg_count = c; total_cost = cost;
            last_refresh = Instant::now();
        }
    }
}

fn ui(f: &mut Frame, tab: usize, models: &[ModelData], sessions: &[SessionData], msg_count: i64, total_cost: f64, scroll: usize, selected: usize) {
    let area = f.area();

    // Header
    let header_chunks = Layout::vertical([Constraint::Length(1), Constraint::Length(3), Constraint::Min(0)]).split(area);

    let mut total_in: i64 = 0; let mut total_out: i64 = 0; let mut total_cr: i64 = 0;
    for m in models { total_in += m.input; total_out += m.output; total_cr += m.cache_read; }
    let total_used = total_in + total_out;
    let chr = if total_in + total_cr > 0 { total_cr as f64 / (total_in + total_cr) as f64 } else { 0.0 };

    let header = Line::from(vec![
        Span::styled(format!(" Tokens: {} ", fmt(total_used)), Style::default().fg(Color::White).bold()),
        Span::styled(format!("│ In: {} ", fmt(total_in)), Style::default().fg(Color::Cyan)),
        Span::styled(format!("Out: {} ", fmt(total_out)), Style::default().fg(Color::Magenta)),
        Span::styled(format!("Cache: {} ", fmt(total_cr)), Style::default().fg(Color::Green)),
        Span::styled(format!("│ CHR: {:.1}% ", chr * 100.0), Style::default().fg(Color::Yellow)),
        Span::styled(format!("Cost: ${:.2} ", total_cost), Style::default().fg(Color::LightGreen)),
        Span::styled(format!("│ {} msgs ", fmt(msg_count)), Style::default().fg(Color::Gray)),
    ]);
    f.render_widget(Paragraph::new(header).bg(Color::Rgb(22,27,34)), header_chunks[0]);

    // Tabs
    let tabs = Tabs::new(vec![" Overview ", " Models ", " Sessions "])
        .select(tab)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan).bold());
    f.render_widget(tabs, header_chunks[1]);

    match tab {
        0 => render_overview(f, header_chunks[2], models, total_in, total_out, total_cr, chr, total_cost, msg_count),
        1 => render_models_tab(f, header_chunks[2], models, scroll, selected),
        2 => render_sessions_tab(f, header_chunks[2], sessions, scroll, selected),
        _ => {}
    }

    // Footer
    let footer = Line::from(vec![
        Span::styled(" q quit ", Style::default().fg(Color::DarkGray)),
        Span::styled(" 1/2/3 tabs ", Style::default().fg(Color::DarkGray)),
        Span::styled(" ↑↓ nav ", Style::default().fg(Color::DarkGray)),
        Span::styled(" r refresh ", Style::default().fg(Color::DarkGray)),
        Span::styled(format!(" auto-refresh {}s ", REFRESH_SECS), Style::default().fg(Color::DarkGray)),
    ]);
    let footer_area = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area)[1];
    f.render_widget(Paragraph::new(footer).bg(Color::Rgb(22,27,34)), footer_area);
}

fn render_overview(f: &mut Frame, area: Rect, models: &[ModelData], total_in: i64, total_out: i64, total_cr: i64, chr: f64, total_cost: f64, msg_count: i64) {
    let chunks = Layout::vertical([Constraint::Length(8), Constraint::Length(8), Constraint::Min(0)]).split(area);

    // Cards row
    let cards = Layout::horizontal([Constraint::Ratio(1,4);4]).split(chunks[0]);

    fn card(title: &str, val: String, sub: String, color: Color) -> Paragraph {
        let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(Color::Rgb(48,54,61)));
        Paragraph::new(vec![
            Line::from(Span::styled(title.to_string(), Style::default().fg(Color::Gray))),
            Line::from(Span::styled(val, Style::default().fg(color).bold())),
            Line::from(Span::styled(sub, Style::default().fg(Color::DarkGray))),
        ]).block(block)
    }

    let total_used = total_in + total_out + models.iter().map(|m| m.cache_write).sum::<i64>();
    let tok_str = fmt(total_used);
    let cost_str = format!("${:.2}", total_cost);
    let chr_str = format!("{:.1}%", chr*100.0);
    let msg_str = fmt(msg_count);
    let model_cnt_str = format!("{} models", models.len());
    f.render_widget(card("Tokens", tok_str, "total".into(), Color::White), cards[0]);
    f.render_widget(card("Cost", cost_str, "estimated".into(), Color::LightGreen), cards[1]);
    f.render_widget(card("Cache Hit", chr_str, "read/total".into(), Color::Yellow), cards[2]);
    f.render_widget(card("Messages", msg_str, model_cnt_str, Color::Cyan), cards[3]);

    // Token breakdown bar
    let max_tok = (total_in + total_out + total_cr).max(1) as f64;
    let bar_chunks = Layout::horizontal([
        Constraint::Ratio((total_in as f64 / max_tok * 100.0) as u32, 100),
        Constraint::Ratio((total_out as f64 / max_tok * 100.0) as u32, 100),
        Constraint::Ratio((total_cr as f64 / max_tok * 100.0) as u32, 100),
    ]).split(chunks[1]);

    let in_bar = Gauge::default().gauge_style(Style::default().fg(Color::Cyan).bg(Color::Cyan)).ratio(1.0).label("Input");
    let out_bar = Gauge::default().gauge_style(Style::default().fg(Color::Magenta).bg(Color::Magenta)).ratio(1.0).label("Output");
    let cache_bar = Gauge::default().gauge_style(Style::default().fg(Color::Green).bg(Color::Green)).ratio(1.0).label("Cache");
    f.render_widget(in_bar, bar_chunks[0]);
    f.render_widget(out_bar, bar_chunks[1]);
    f.render_widget(cache_bar, bar_chunks[2]);

    // Top models mini table
    let rows: Vec<Row> = models.iter().take(10).map(|m| {
        Row::new(vec![
            Cell::from(m.model_id.as_str()),
            Cell::from(fmt(m.input+m.output)).style(Style::default().fg(Color::White)),
            Cell::from(fmt(m.cache_read)).style(Style::default().fg(Color::Green)),
            Cell::from(format!("${:.2}", m.cost)).style(Style::default().fg(Color::LightGreen)),
        ])
    }).collect();

    let w = [Constraint::Percentage(40), Constraint::Percentage(22), Constraint::Percentage(22), Constraint::Percentage(16)];
    let table = Table::new(rows, w)
        .header(Row::new(vec!["Model","Tokens","Cache","Cost"]).style(Style::default().fg(Color::DarkGray)))
        .block(Block::default().borders(Borders::ALL).title("Top Models").border_style(Style::default().fg(Color::Rgb(48,54,61))));
    f.render_widget(table, chunks[2]);
}

fn render_models_tab(f: &mut Frame, area: Rect, models: &[ModelData], scroll: usize, selected: usize) {
    let count = models.len();
    let end = (scroll + 20).min(count);
    let rows: Vec<Row> = models[scroll..end].iter().enumerate().map(|(i, m)| {
        let style = if scroll + i == selected { Style::default().bg(Color::Rgb(30,40,60)) } else { Style::default() };
        Row::new(vec![
            Cell::from(m.model_id.as_str()),
            Cell::from(format!("{}", m.sessions)).style(Style::default()),
            Cell::from(fmt(m.input)).style(Style::default().fg(Color::Cyan)),
            Cell::from(fmt(m.output)).style(Style::default().fg(Color::Magenta)),
            Cell::from(fmt(m.cache_read)).style(Style::default().fg(Color::Green)),
            Cell::from(format!("${:.2}", m.cost)).style(Style::default().fg(Color::LightGreen)),
        ]).style(style)
    }).collect();

    let w = [Constraint::Percentage(32), Constraint::Percentage(10), Constraint::Percentage(16), Constraint::Percentage(16), Constraint::Percentage(16), Constraint::Percentage(10)];
    let t = Table::new(rows, w)
        .header(Row::new(vec!["Model","Sess","Input","Output","Cache","Cost"]).style(Style::default().fg(Color::Cyan)))
        .block(Block::default().borders(Borders::ALL).title(format!("Models ({})", count)));
    f.render_widget(t, area);
}

fn render_sessions_tab(f: &mut Frame, area: Rect, sessions: &[SessionData], scroll: usize, selected: usize) {
    let count = sessions.len();
    let end = (scroll + 20).min(count);
    let rows: Vec<Row> = sessions[scroll..end].iter().enumerate().map(|(i, s)| {
        let style = if scroll + i == selected { Style::default().bg(Color::Rgb(30,40,60)) } else { Style::default() };
        Row::new(vec![
            Cell::from(format!("{}…", &s.session_id[..8.min(s.session_id.len())])).style(Style::default().fg(Color::Cyan)),
            Cell::from(s.client.as_str()),
            Cell::from(s.model_id.as_str()),
            Cell::from(fmt(s.input + s.output)).style(Style::default()),
            Cell::from(fmt(s.messages as i64)).style(Style::default()),
            Cell::from(format!("${:.2}", s.cost)).style(Style::default().fg(Color::LightGreen)),
        ]).style(style)
    }).collect();

    let w = [Constraint::Percentage(20), Constraint::Percentage(12), Constraint::Percentage(28), Constraint::Percentage(18), Constraint::Percentage(10), Constraint::Percentage(12)];
    let t = Table::new(rows, w)
        .header(Row::new(vec!["Session","Client","Model","Tokens","Msgs","Cost"]).style(Style::default().fg(Color::Cyan)))
        .block(Block::default().borders(Borders::ALL).title(format!("Sessions ({})", count)));
    f.render_widget(t, area);
}

fn fmt(n: i64) -> String {
    if n >= 1_000_000_000 { format!("{:.1}B", n as f64 / 1e9) }
    else if n >= 1_000_000 { format!("{:.1}M", n as f64 / 1e6) }
    else if n >= 1_000 { format!("{:.0}K", n as f64 / 1e3) }
    else { n.to_string() }
}
