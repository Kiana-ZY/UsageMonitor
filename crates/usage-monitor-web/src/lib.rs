//! UsageMonitor web dashboard.
//!
//! axum HTTP server with REST API + embedded static frontend.

use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use tower_http::cors::CorsLayer;
use usage_monitor_core::DataSource;
use usage_monitor_cc_switch::CcSwitchAdapter;
use usage_monitor_pricing::PricingEngine;
use usage_monitor_storage::Storage;

#[derive(Clone)]
struct AppState {
    storage: Arc<Storage>,
    pricing: Arc<PricingEngine>,
    db_path: PathBuf,
    home: PathBuf,
}

#[derive(Serialize)]
struct SummaryResponse {
    total_input: i64,
    total_output: i64,
    total_cache_read: i64,
    total_cache_write: i64,
    cache_hit_rate: f64,
    total_cost: f64,
    message_count: i64,
    tool_count: usize,
    models: Vec<serde_json::Value>,
    daily: Vec<serde_json::Value>,
}

pub async fn serve(db_path: PathBuf, home: PathBuf, start_port: u16) -> anyhow::Result<()> {
    let cc_db = home.join(".cc-switch").join("cc-switch.db");
    let pricing = Arc::new(PricingEngine::new(
        if cc_db.exists() { cc_db.to_str() } else { None },
    ));

    std::fs::create_dir_all(db_path.parent().unwrap())?;
    let storage = Arc::new(Storage::open(&db_path)?);

    let state = AppState {
        storage,
        pricing,
        db_path,
        home,
    };

    let app = Router::new()
        .route("/", get(index_page))
        .route("/api/summary", get(api_summary))
        .route("/api/models", get(api_models))
        .route("/api/sessions", get(api_sessions))
        .route("/api/daily", get(api_daily))
        .route("/api/scan", post(api_scan))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // Try ports from start_port upwards
    let listener = bind_port(start_port).await?;
    let actual_port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{}", actual_port);
    println!();
    println!("  UsageMonitor Dashboard");
    println!("  ─────────────────────");
    println!("  Local:   {}", url);
    println!("  Press Ctrl+C to stop");
    println!();

    // Auto-open browser
    let url_clone = url.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = open::that(&url_clone);
    });

    axum::serve(listener, app).await?;
    Ok(())
}

async fn bind_port(start: u16) -> anyhow::Result<tokio::net::TcpListener> {
    for port in start..start + 10 {
        let addr = format!("127.0.0.1:{}", port);
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(l) => return Ok(l),
            Err(_) => continue,
        }
    }
    Err(anyhow::anyhow!("Could not bind to any port in range {}-{}", start, start + 10))
}

async fn index_page() -> Html<&'static str> {
    Html(include_str!("dashboard.html"))
}

async fn api_summary(State(state): State<AppState>) -> Result<Json<serde_json::Value>, StatusCode> {
    let count = state.storage.messages_count().unwrap_or(0);
    let models = state.storage.query_models().unwrap_or_default();
    let daily = state.storage.query_daily().unwrap_or_default();

    let mut total_input = 0i64;
    let mut total_output = 0i64;
    let mut total_cache_read = 0i64;
    let mut total_cache_write = 0i64;
    let mut total_cost = 0.0;
    let mut tools = std::collections::HashSet::new();

    let model_list: Vec<serde_json::Value> = models
        .iter()
        .map(|m| {
            total_input += m.tokens.input;
            total_output += m.tokens.output;
            total_cache_read += m.tokens.cache_read;
            total_cache_write += m.tokens.cache_write;
            tools.insert(m.provider_id.clone());
            serde_json::json!({
                "model_id": m.model_id,
                "provider_id": m.provider_id,
                "input": m.tokens.input,
                "output": m.tokens.output,
                "cache_read": m.tokens.cache_read,
                "sessions": m.session_count,
                "requests": m.request_count,
            })
        })
        .collect();

    let daily_list: Vec<serde_json::Value> = daily
        .iter()
        .map(|d| {
            serde_json::json!({
                "date": d.date,
                "input": d.tokens.input,
                "output": d.tokens.output,
                "cache_read": d.tokens.cache_read,
                "requests": d.request_count,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "total_input": total_input,
        "total_output": total_output,
        "total_cache_read": total_cache_read,
        "total_cache_write": total_cache_write,
        "cache_hit_rate": usage_monitor_core::cache_hit_rate(total_cache_read, total_input),
        "total_cost": total_cost,
        "message_count": count,
        "tool_count": tools.len(),
        "models": model_list,
        "daily": daily_list,
    })))
}

async fn api_models(State(state): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let models = state.storage.query_models().unwrap_or_default();
    let res: Vec<serde_json::Value> = models.iter().map(|m| serde_json::json!({
        "model_id": m.model_id,
        "provider_id": m.provider_id,
        "input": m.tokens.input,
        "output": m.tokens.output,
        "cache_read": m.tokens.cache_read,
        "cache_write": m.tokens.cache_write,
        "cost": m.cost,
        "sessions": m.session_count,
        "requests": m.request_count,
        "estimated_cost": state.pricing.calculate_cost(&m.model_id, &m.tokens),
    })).collect();
    Json(res)
}

async fn api_sessions(State(state): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let sessions = state.storage.query_sessions().unwrap_or_default();
    let res: Vec<serde_json::Value> = sessions.iter().map(|s| serde_json::json!({
        "session_id": s.session_id,
        "client": s.client,
        "model_id": s.model_id,
        "input": s.tokens.input,
        "output": s.tokens.output,
        "cache_read": s.tokens.cache_read,
        "messages": s.message_count,
        "first_seen": s.first_seen,
        "last_seen": s.last_seen,
        "estimated_cost": state.pricing.calculate_cost(&s.model_id, &s.tokens),
    })).collect();
    Json(res)
}

async fn api_daily(State(state): State<AppState>) -> Json<Vec<serde_json::Value>> {
    let daily = state.storage.query_daily().unwrap_or_default();
    let res: Vec<serde_json::Value> = daily.iter().map(|d| serde_json::json!({
        "date": d.date,
        "input": d.tokens.input,
        "output": d.tokens.output,
        "cache_read": d.tokens.cache_read,
        "requests": d.request_count,
        "cost": d.cost,
    })).collect();
    Json(res)
}

async fn api_scan(State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut all_messages = Vec::new();

    // CC Switch
    let cc_switch = CcSwitchAdapter::new(
        state.home.join(".cc-switch").join("cc-switch.db"),
    );
    if cc_switch.enabled() {
        if let Ok(msgs) = cc_switch.collect() {
            all_messages.extend(msgs);
        }
    }

    // Native Claude Code
    let claude_dir = state.home.join(".claude").join("projects");
    let native = usage_monitor_parsers::parse_all(
        if claude_dir.exists() { Some(claude_dir.as_path()) } else { None },
        None, None, None, None, None, None,
    );
    all_messages.extend(native);

    let inserted = state.storage.insert_messages(&all_messages).unwrap_or(0);
    state.storage.upsert_daily_rollups().ok();

    Json(serde_json::json!({
        "status": "ok",
        "total": all_messages.len(),
        "inserted": inserted,
    }))
}
