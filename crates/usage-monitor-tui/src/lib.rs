// UsageMonitor TUI — adapted from Tokscale's ratatui architecture (MIT)

mod bar_chart;
mod widgets;

use std::collections::{BTreeMap, HashMap};
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
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs},
    Frame, Terminal,
};

use bar_chart::{ModelSegment, StackedBarData, render_stacked_bar_chart};
use widgets::{format_tokens, format_cost, format_cache_hit_rate, get_provider_shade, get_model_color, get_provider_from_model};

use usage_monitor_pricing::PricingEngine;
use usage_monitor_storage::Storage;

const REFRESH_SECS: u64 = 15;
const BG: Color = Color::Rgb(13,17,23);
const BORDER: Color = Color::Rgb(48,54,61);
const MUTED: Color = Color::Rgb(139,148,158);
const DIM: Color = Color::Rgb(72,79,88);

// ── Data ──

struct App {
    tab: usize,
    scroll: usize,
    selected: usize,
    sort_col: u8,
    sort_desc: bool,
    last_refresh: Instant,
    models: Vec<ModelRow>,
    daily: Vec<DailyRow>,
    sessions: Vec<SessRow>,
    heatmap: Vec<(String, i64)>,
    msg_count: i64,
    total_cost: f64,
    total_input: i64, total_output: i64, total_cache_read: i64, total_cache_write: i64,
    model_shade: HashMap<String, (Color, usize)>,
}

struct ModelRow { model_id: String, provider_id: String, input: i64, output: i64, cache_read: i64, cache_write: i64, sessions: usize, requests: usize, cost: f64 }
struct DailyRow { date: String, total: i64, input: i64, output: i64, cache_read: i64, cache_write: i64, cost: f64, requests: usize, models: Vec<(String, i64)> }
struct SessRow { session_id: String, client: String, model_id: String, tokens: i64, cache_read: i64, messages: usize, cost: f64 }

impl App {
    fn load(storage: &Storage, pricing: &PricingEngine) -> Self {
        let mut a = Self { tab:0,scroll:0,selected:0,sort_col:4,sort_desc:true,last_refresh:Instant::now(),
            models:vec![],daily:vec![],sessions:vec![],heatmap:vec![],msg_count:0,total_cost:0.0,
            total_input:0,total_output:0,total_cache_read:0,total_cache_write:0,model_shade:HashMap::new() };
        a.reload(storage, pricing);
        a
    }

    fn reload(&mut self, storage: &Storage, pricing: &PricingEngine) {
        let ms = storage.query_models().unwrap_or_default();
        let ss = storage.query_sessions().unwrap_or_default();
        self.msg_count = storage.messages_count().unwrap_or(0);

        // Model→shade mapping (ranked by total tokens)
        let mut ranked: Vec<(String, i64)> = ms.iter().map(|m| (m.model_id.clone(), m.tokens.input+m.tokens.output+m.tokens.cache_read)).collect();
        ranked.sort_by(|a,b| b.1.cmp(&a.1));
        self.model_shade.clear();
        for (i,(name,_)) in ranked.iter().enumerate() {
            let provider = get_provider_from_model(name);
            self.model_shade.insert(name.clone(), (get_provider_shade(provider, i / 5), i));
        }

        self.models = ms.iter().map(|x| {
            let (color,_) = self.model_shade.get(&x.model_id).copied().unwrap_or((Color::Gray,0));
            ModelRow { model_id:x.model_id.clone(),provider_id:x.provider_id.clone(),
                input:x.tokens.input,output:x.tokens.output,cache_read:x.tokens.cache_read,cache_write:x.tokens.cache_write,
                sessions:x.session_count,requests:x.request_count,cost:pricing.calculate_cost(&x.model_id,&x.tokens) }
        }).collect();

        self.total_input = self.models.iter().map(|m|m.input).sum();
        self.total_output = self.models.iter().map(|m|m.output).sum();
        self.total_cache_read = self.models.iter().map(|m|m.cache_read).sum();
        self.total_cache_write = self.models.iter().map(|m|m.cache_write).sum();
        self.total_cost = self.models.iter().map(|m|m.cost).sum();

        self.sessions = ss.iter().map(|x| SessRow { session_id:x.session_id.clone(),client:x.client.clone(),
            model_id:x.model_id.clone(),tokens:x.tokens.input+x.tokens.output,cache_read:x.tokens.cache_read,
            messages:x.message_count,cost:pricing.calculate_cost(&x.model_id,&x.tokens) }).collect();

        // Daily with per-model breakdown
        let conn = storage.lock();
        let mut stmt = conn.prepare("SELECT date(timestamp/1000,'unixepoch','localtime'),model_id,SUM(input_tokens),SUM(output_tokens),SUM(cache_read_tokens),SUM(cache_write_tokens),COUNT(*),SUM(cost_usd) FROM messages GROUP BY 1,2 ORDER BY 1").unwrap();
        let mut day_map: BTreeMap<String,DailyRow> = BTreeMap::new();
        for row in stmt.query_map([],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,i64>(5)?,r.get::<_,i64>(6)?,r.get::<_,f64>(7)?))).unwrap().flatten() {
            let (date,model,inp,out,cr,cw,reqs,cost) = row;
            let e = day_map.entry(date.clone()).or_insert_with(|| DailyRow{date,total:0,input:0,output:0,cache_read:0,cache_write:0,cost:0.0,requests:0,models:vec![]});
            e.input+=inp;e.output+=out;e.cache_read+=cr;e.cache_write+=cw;e.cost+=cost;e.requests+=reqs as usize;
            e.total+=inp+out+cr;e.models.push((model,inp+out+cr));
        }
        self.daily = day_map.into_values().collect();
        for d in &mut self.daily { d.models.sort_by(|a,b| b.1.cmp(&a.1)); }
        self.heatmap = self.daily.iter().map(|d| (d.date.clone(), d.total)).collect();
        self.sort();
    }

    fn sort(&mut self) {
        match self.sort_col {
            0=>self.models.sort_by(|a,b|if self.sort_desc{b.model_id.cmp(&a.model_id)}else{a.model_id.cmp(&b.model_id)}),
            1=>self.models.sort_by(|a,b|if self.sort_desc{b.sessions.cmp(&a.sessions)}else{a.sessions.cmp(&b.sessions)}),
            2=>self.models.sort_by(|a,b|if self.sort_desc{(b.input+b.output).cmp(&(a.input+a.output))}else{(a.input+a.output).cmp(&(b.input+b.output))}),
            3=>self.models.sort_by(|a,b|if self.sort_desc{b.cache_read.cmp(&a.cache_read)}else{a.cache_read.cmp(&b.cache_read)}),
            _=>self.models.sort_by(|a,b|if self.sort_desc{b.cost.partial_cmp(&a.cost).unwrap()}else{a.cost.partial_cmp(&b.cost).unwrap()}),
        }
    }

    fn scroll_to(&mut self) {
        if self.selected<self.scroll{self.scroll=self.selected}
        if self.selected>=self.scroll.saturating_add(18){self.scroll=self.selected.saturating_sub(17)}
    }
}

// ── Entry ──

pub fn run(db_path: PathBuf, home: PathBuf) -> anyhow::Result<()> {
    let storage = Storage::open(&db_path)?;
    let cc_db = home.join(".cc-switch").join("cc-switch.db");
    let pricing = PricingEngine::new(if cc_db.exists(){cc_db.to_str()}else{None});
    let mut app = App::load(&storage, &pricing);

    enable_raw_mode()?; let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut t = Terminal::new(backend)?;
    let res = event_loop(&mut t, &mut app, &storage, &pricing);
    disable_raw_mode()?; execute!(t.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    t.show_cursor()?; res
}

fn event_loop<B: Backend>(t: &mut Terminal<B>, app: &mut App, storage: &Storage, pricing: &PricingEngine) -> anyhow::Result<()> {
    loop {
        t.draw(|f| ui(f, app))?;
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(k) if k.kind != KeyEventKind::Release => { if handle_key(app, k.code) { return Ok(()); } }
                Event::Mouse(m) => match m.kind {
                    MouseEventKind::ScrollDown => { app.scroll+=3; app.selected+=3; }
                    MouseEventKind::ScrollUp => { app.scroll=app.scroll.saturating_sub(3); app.selected=app.selected.saturating_sub(3); }
                    _ => {}
                },
                _ => {}
            }
        }
        if app.last_refresh.elapsed().as_secs() >= REFRESH_SECS { app.reload(storage, pricing); app.last_refresh = Instant::now(); }
    }
}

fn handle_key(app: &mut App, key: KeyCode) -> bool {
    match key {
        KeyCode::Char('q')|KeyCode::Esc => return true,
        KeyCode::Char('1')=>{app.tab=0;app.scroll=0;app.selected=0} KeyCode::Char('2')=>{app.tab=1;app.scroll=0;app.selected=0}
        KeyCode::Char('3')=>{app.tab=2;app.scroll=0;app.selected=0} KeyCode::Char('4')=>{app.tab=3;app.scroll=0;app.selected=0}
        KeyCode::Right|KeyCode::Char('l')=>{app.tab=(app.tab+1)%4;app.scroll=0;app.selected=0}
        KeyCode::Left|KeyCode::Char('h')=>{app.tab=if app.tab==0{3}else{app.tab-1};app.scroll=0;app.selected=0}
        KeyCode::Tab=>{app.tab=(app.tab+1)%4;app.scroll=0;app.selected=0}
        KeyCode::Char('s')=>{app.sort_col=(app.sort_col+1)%5;app.sort_desc=!app.sort_desc;app.sort()}
        KeyCode::Char('r')=>{app.last_refresh=Instant::now()-Duration::from_secs(REFRESH_SECS)}
        KeyCode::Up|KeyCode::Char('k')=>{app.selected=app.selected.saturating_sub(1);app.scroll_to()}
        KeyCode::Down|KeyCode::Char('j')=>{app.selected+=1;app.scroll_to()}
        KeyCode::PageUp=>{app.scroll=app.scroll.saturating_sub(10);app.selected=app.selected.saturating_sub(10)}
        KeyCode::PageDown=>{app.scroll+=10;app.selected+=10}
        KeyCode::Home=>{app.scroll=0;app.selected=0}
        KeyCode::End=>{app.scroll=usize::MAX;app.selected=usize::MAX}
        _=>{}
    }
    false
}

// ── UI ──

fn ui(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::vertical([Constraint::Length(1),Constraint::Length(3),Constraint::Min(0),Constraint::Length(1)]).split(area);

    // Header
    let used = app.total_input + app.total_output + app.total_cache_write;
    let chr = if app.total_input+app.total_cache_read>0 { app.total_cache_read as f64/(app.total_input+app.total_cache_read) as f64 } else {0.0};
    let hdr = Line::from(vec![
        Span::styled(" UsageMonitor ", Style::default().fg(Color::White).bold()),
        Span::styled(format!("│ {} ", format_tokens(used as u64)), Style::default().fg(Color::Cyan)),
        Span::styled(format!("{} ", format_cost(app.total_cost)), Style::default().fg(Color::Green)),
        Span::styled(format!("CHR {:.1}% ", chr*100.0), Style::default().fg(Color::Yellow)),
        Span::styled(format!("{} msgs ", app.msg_count), Style::default().fg(MUTED)),
    ]);
    f.render_widget(Paragraph::new(hdr).bg(BG), chunks[0]);

    // Tabs
    let tabs = Tabs::new(vec![" Overview "," Models "," Daily "," Stats "]).select(app.tab)
        .style(Style::default().fg(DIM)).highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan).bold());
    f.render_widget(tabs, chunks[1]);

    match app.tab { 0=>tab_overview(f,chunks[2],app), 1=>tab_models(f,chunks[2],app), 2=>tab_daily(f,chunks[2],app), 3=>tab_stats(f,chunks[2],app), _=>{} }

    // Footer
    let ft = Line::from(vec![
        Span::styled(" ←→/h/l/1-4 tabs ", Style::default().fg(DIM)),
        Span::styled(" ↑↓/j/k nav ", Style::default().fg(DIM)),
        Span::styled(" s sort ", Style::default().fg(DIM)),
        Span::styled(" r refresh ", Style::default().fg(DIM)),
        Span::styled(" q quit ", Style::default().fg(DIM)),
    ]);
    f.render_widget(Paragraph::new(ft).bg(BG), chunks[3]);
}

// ── Overview (Tokscale-style: stacked chart + legend + top models) ──

fn tab_overview(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(8), Constraint::Length(1), Constraint::Min(16), Constraint::Min(0)]).split(area);

    // KPI cards
    let cards = Layout::horizontal([Constraint::Ratio(1,4);4]).split(rows[0]);
    let used = app.total_input + app.total_output + app.total_cache_write;
    let chr = if app.total_input+app.total_cache_read>0 { app.total_cache_read as f64/(app.total_input+app.total_cache_read) as f64 } else {0.0};
    let kpi = [
        ("Tokens", format_tokens(used as u64), format!("in {} · out {}", format_tokens(app.total_input as u64), format_tokens(app.total_output as u64)), Color::White),
        ("Cost", format_cost(app.total_cost), "estimated".into(), Color::Green),
        ("Cache Hit", format!("{:.1}%", chr*100.0), format_cache_hit_rate(app.total_cache_read as u64, app.total_input as u64, app.total_cache_write as u64), Color::Yellow),
        ("Models", app.models.len().to_string(), format!("{} msgs", app.msg_count), Color::Cyan),
    ];
    for (i,(l,v,s,c)) in kpi.iter().enumerate() {
        let b = Block::default().borders(Borders::ALL).border_style(Style::default().fg(BORDER));
        f.render_widget(Paragraph::new(vec![
            Line::from(Span::styled(l.to_string(), Style::default().fg(MUTED))),
            Line::from(Span::styled(v.clone(), Style::default().fg(*c).bold())),
            Line::from(Span::styled(s.clone(), Style::default().fg(DIM))),
        ]).block(b), cards[i]);
    }

    // Legend
    let mut sorted: Vec<_> = app.model_shade.iter().collect();
    sorted.sort_by(|a,b| a.1.1.cmp(&b.1.1));
    let legend: Vec<Span> = sorted.iter().take(8).flat_map(|(name,(c,_))| vec![
        Span::styled("● ", Style::default().fg(*c)),
        Span::styled(format!("{} ", name), Style::default().fg(DIM)),
    ]).collect();
    f.render_widget(Paragraph::new(Line::from(legend)), rows[1]);

    // Stacked bar chart
    let main = Layout::horizontal([Constraint::Ratio(3,5), Constraint::Ratio(2,5)]).split(rows[2]);
    let bar_data: Vec<StackedBarData> = app.daily.iter().rev().take(60).map(|d| StackedBarData {
        date: format!("{}/{}", &d.date[5..7], &d.date[8..10]),
        total: d.total as u64,
        models: d.models.iter().map(|(name,tok)| {
            let (color,_) = app.model_shade.get(name).copied().unwrap_or((Color::Gray,0));
            ModelSegment { model_id: name.clone(), tokens: *tok as u64, color }
        }).collect(),
    }).collect();
    render_stacked_bar_chart(f, main[0], &bar_data, MUTED, Color::Cyan);

    // Top models
    render_top_models(f, main[1], app);
}

fn render_top_models(f: &mut Frame, area: Rect, app: &App) {
    let rows: Vec<Row> = app.models.iter().take(14).map(|m| {
        let (color,_) = app.model_shade.get(&m.model_id).copied().unwrap_or((Color::Gray,0));
        Row::new(vec![
            Cell::from(format!("● {}", m.model_id)).style(Style::default().fg(color)),
            Cell::from(format_tokens((m.input+m.output) as u64)).style(Style::default().fg(Color::White)),
            Cell::from(format_tokens(m.cache_read as u64)).style(Style::default().fg(Color::Green)),
            Cell::from(format_cost(m.cost)).style(Style::default().fg(Color::Green)),
        ])
    }).collect();
    let w = [Constraint::Percentage(40),Constraint::Percentage(22),Constraint::Percentage(22),Constraint::Percentage(16)];
    let t = Table::new(rows, w)
        .header(Row::new(vec!["Model","Tokens","Cache","Cost"]).style(Style::default().fg(DIM)))
        .block(Block::default().borders(Borders::ALL).title(" Top Models ").border_style(Style::default().fg(BORDER)));
    f.render_widget(t, area);
}

// ── Models ──

fn tab_models(f: &mut Frame, area: Rect, app: &App) {
    let end = (app.scroll+22).min(app.models.len());
    let rows: Vec<Row> = app.models[app.scroll..end].iter().map(|m| {
        let (color,_) = app.model_shade.get(&m.model_id).copied().unwrap_or((Color::Gray,0));
        Row::new(vec![
            Cell::from(format!("● {}", m.model_id)).style(Style::default().fg(color)),
            Cell::from(format!("{}",m.sessions)), Cell::from(format_tokens((m.input+m.output) as u64)),
            Cell::from(format_tokens(m.cache_read as u64)).style(Style::default().fg(Color::Green)),
            Cell::from(format_cost(m.cost)),
        ])
    }).collect();
    let w = [Constraint::Percentage(36),Constraint::Percentage(10),Constraint::Percentage(20),Constraint::Percentage(18),Constraint::Percentage(16)];
    let t = Table::new(rows,w).header(Row::new(vec!["Model","Sess","Tokens","Cache","Cost"]).style(Style::default().fg(Color::Cyan)))
        .block(Block::default().borders(Borders::ALL).title(format!(" Models ({}) s:sort ",app.models.len())));
    f.render_widget(t, area);
}

// ── Daily ──

fn tab_daily(f: &mut Frame, area: Rect, app: &App) {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let end = (app.scroll+22).min(app.daily.len());
    let rows: Vec<Row> = app.daily.iter().rev().skip(app.scroll).take(22).map(|d| {
        let is = d.date == today;
        Row::new(vec![
            Cell::from(d.date.clone()).style(if is{Style::default().fg(Color::Yellow).bold()}else{Style::default()}),
            Cell::from(format_tokens(d.input as u64)).style(Style::default().fg(Color::Cyan)),
            Cell::from(format_tokens(d.output as u64)).style(Style::default().fg(Color::Magenta)),
            Cell::from(format_tokens(d.cache_read as u64)).style(Style::default().fg(Color::Green)),
            Cell::from(format_tokens(d.cache_write as u64)),
            Cell::from(format!("{}",d.requests)), Cell::from(format_cost(d.cost)),
        ])
    }).collect();
    let w = [Constraint::Percentage(18),Constraint::Percentage(16),Constraint::Percentage(16),Constraint::Percentage(16),Constraint::Percentage(16),Constraint::Percentage(10),Constraint::Percentage(8)];
    let t = Table::new(rows,w).header(Row::new(vec!["Date","Input","Output","Cache R","Cache W","Reqs","Cost"]).style(Style::default().fg(Color::Cyan)))
        .block(Block::default().borders(Borders::ALL).title(format!(" Daily ({}) ",app.daily.len())));
    f.render_widget(t, area);
}

// ── Stats ──

fn tab_stats(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::vertical([Constraint::Length(8),Constraint::Min(0)]).split(area);
    let cards = Layout::horizontal([Constraint::Ratio(1,4);4]).split(chunks[0]);

    let mut dates: Vec<String> = app.heatmap.iter().map(|(d,_)| d.clone()).collect();
    dates.sort(); dates.dedup();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let streak = calc_streak(&dates, &today);
    let longest = calc_longest(&dates);
    let active = dates.len();
    let total_days = if let (Some(f),Some(l)) = (dates.first(),dates.last()) {
        (chrono::NaiveDate::parse_from_str(l,"%Y-%m-%d").unwrap()-chrono::NaiveDate::parse_from_str(f,"%Y-%m-%d").unwrap()).num_days() as usize+1
    } else {1};

    let sd = [(format!("{} days",streak),"Current Streak",Color::Cyan),(format!("{} days",longest),"Longest Streak",Color::Yellow),(format!("{}/{}",active,total_days),"Active Days",Color::White),(format_cost(app.total_cost),"Total Cost",Color::Green)];
    for (i,(v,l,c)) in sd.iter().enumerate() {
        let b = Block::default().borders(Borders::ALL).border_style(Style::default().fg(BORDER));
        f.render_widget(Paragraph::new(vec![
            Line::from(Span::styled(l.to_string(),Style::default().fg(MUTED))),
            Line::from(Span::styled(v.clone(),Style::default().fg(*c).bold())),
        ]).block(b), cards[i]);
    }

    // Text heatmap
    if !app.heatmap.is_empty() {
        let by_date: HashMap<String,i64> = app.heatmap.iter().cloned().collect();
        let max_v = by_date.values().max().copied().unwrap_or(1).max(1);
        let cols = 26; let end = chrono::Local::now().date_naive();
        let start = end - chrono::Duration::days(cols*7-1);
        let mut lines = vec![];
        for row in 0..7 {
            let mut spans = vec![];
            for col in 0..cols {
                let ds = (start+chrono::Duration::days((col*7+row) as i64)).format("%Y-%m-%d").to_string();
                let v = by_date.get(&ds).copied().unwrap_or(0);
                let (ch,c) = if v==0{('·',Color::Rgb(22,27,34))} else if v<max_v/3{('░',Color::Rgb(14,68,41))} else if v<max_v*2/3{('▒',Color::Rgb(0,109,50))} else {('▓',Color::Rgb(57,211,83))};
                spans.push(Span::styled(format!("{} ",ch), Style::default().fg(c)));
            }
            lines.push(Line::from(spans));
        }
        f.render_widget(Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Contribution Heatmap ")), chunks[1]);
    }
}

fn calc_streak(dates: &[String], today: &str) -> usize {
    let mut s=0; let mut d=chrono::NaiveDate::parse_from_str(today,"%Y-%m-%d").unwrap();
    loop { if dates.contains(&d.format("%Y-%m-%d").to_string()){s+=1;d-=chrono::Duration::days(1)} else {break} } s
}
fn calc_longest(dates: &[String]) -> usize {
    let mut sorted: Vec<chrono::NaiveDate> = dates.iter().filter_map(|d|chrono::NaiveDate::parse_from_str(d,"%Y-%m-%d").ok()).collect();
    sorted.sort();sorted.dedup();
    let mut longest=0;let mut cur=0;let mut prev:Option<chrono::NaiveDate>=None;
    for d in sorted{if let Some(p)=prev{if d==p+chrono::Duration::days(1){cur+=1}else{cur=1}}else{cur=1};longest=longest.max(cur);prev=Some(d)}
    longest
}
