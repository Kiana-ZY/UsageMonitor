use std::io;
use std::path::PathBuf;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table, Tabs},
    Frame, Terminal,
};
use usage_monitor_pricing::PricingEngine;
use usage_monitor_storage::Storage;

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
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    res
}

fn app_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    storage: &Storage,
    pricing: &PricingEngine,
) -> anyhow::Result<()> {
    let mut tab: usize = 0;
    let mut scroll: usize = 0;

    loop {
        terminal.draw(|f| ui(f, storage, pricing, tab, scroll))?;
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Tab => tab = (tab + 1) % 2,
                KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => scroll += 1,
                _ => {}
            }
        }
    }
}

fn ui(f: &mut Frame, storage: &Storage, pricing: &PricingEngine, tab: usize, _scroll: usize) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(f.area());

    let tabs = Tabs::new(vec![" Overview ", " Models "])
        .select(tab)
        .style(Style::default().fg(Color::Cyan))
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan));
    f.render_widget(tabs, chunks[0]);

    match tab {
        0 => render_overview(f, chunks[1], storage, pricing),
        1 => render_models(f, chunks[1], storage, pricing),
        _ => {}
    }
}

fn render_overview(f: &mut Frame, area: ratatui::layout::Rect, storage: &Storage, pricing: &PricingEngine) {
    let models = storage.query_models().unwrap_or_default();
    let count = storage.messages_count().unwrap_or(0);

    let mut total_input = 0i64;
    let mut total_output = 0i64;
    let mut total_cache_read = 0i64;
    let mut total_cost = 0.0;

    for m in &models {
        total_input += m.tokens.input;
        total_output += m.tokens.output;
        total_cache_read += m.tokens.cache_read;
        total_cost += pricing.calculate_cost(&m.model_id, &m.tokens);
    }
    let chr = usage_monitor_core::cache_hit_rate(total_cache_read, total_input);

    let cards = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 4); 4])
        .split(area);

    let card = |title, value, sub| {
        Paragraph::new(vec![
            Line::from(Span::styled(title, Style::default().fg(Color::Gray))),
            Line::from(Span::styled(value, Style::default().fg(Color::White).add_modifier(Modifier::BOLD))),
            Line::from(Span::styled(sub, Style::default().fg(Color::DarkGray))),
        ])
        .block(Block::default().borders(Borders::ALL))
    };

    let inp_str = fmt(total_input);
    let out_str = fmt(total_output);
    let chr_str = format!("{:.1}%", chr * 100.0);
    let msg_str = fmt(count);
    let sub_str = format!("{} models | ${:.2}", models.len(), total_cost);

    f.render_widget(card("Input", inp_str.as_str(), "tokens"), cards[0]);
    f.render_widget(card("Output", out_str.as_str(), "tokens"), cards[1]);
    f.render_widget(card("Cache Hit", chr_str.as_str(), "read / total"), cards[2]);
    f.render_widget(card("Messages", msg_str.as_str(), sub_str.as_str()), cards[3]);
}

fn render_models(f: &mut Frame, area: ratatui::layout::Rect, storage: &Storage, pricing: &PricingEngine) {
    let models = storage.query_models().unwrap_or_default();

    let header = Row::new(vec!["Model", "Input", "Output", "Cache Read", "Cost"])
        .style(Style::default().fg(Color::Cyan));

    let rows: Vec<Row> = models
        .iter()
        .map(|m| {
            let cost = pricing.calculate_cost(&m.model_id, &m.tokens);
            Row::new(vec![
                m.model_id.clone(),
                fmt(m.tokens.input),
                fmt(m.tokens.output),
                fmt(m.tokens.cache_read),
                format!("${:.2}", cost),
            ])
        })
        .collect();

    let widths = [
        Constraint::Percentage(30),
        Constraint::Percentage(18),
        Constraint::Percentage(18),
        Constraint::Percentage(18),
        Constraint::Percentage(16),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title("Models"));

    f.render_widget(table, area);
}

fn fmt(n: i64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.1}B", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.0}K", n as f64 / 1e3)
    } else {
        n.to_string()
    }
}
