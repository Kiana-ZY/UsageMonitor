// Adapted from Tokscale widgets.rs (MIT)
// Provider color palettes + format utilities

use ratatui::style::Color;

pub fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000_000 { format!("{:.1}B", tokens as f64 / 1e9) }
    else if tokens >= 1_000_000 { format!("{:.1}M", tokens as f64 / 1e6) }
    else if tokens >= 1_000 { format!("{}K", tokens / 1000) }
    else { tokens.to_string() }
}

pub fn format_cost(cost: f64) -> String {
    if !cost.is_finite() || cost < 0.0 { return "$0.00".into() }
    if cost >= 1000.0 { format!("${:.1}K", cost / 1000.0) }
    else { format!("${:.2}", cost) }
}

pub fn format_cache_hit_rate(cache_read: u64, input: u64, cache_write: u64) -> String {
    let paid = input.saturating_add(cache_write);
    if paid == 0 { return if cache_read > 0 { "∞".into() } else { "—".into() } }
    format!("{:.1}x", cache_read as f64 / paid as f64)
}

// ── Provider color palettes (7-step shades, Tokscale-style) ──

pub fn get_provider_shade(provider: &str, rank: usize) -> Color {
    let p = provider.to_lowercase();
    let palette: &[(u8,u8,u8)] = if p.contains("anthropic") { &ANTHROPIC_SHADES }
    else if p.contains("openai") { &OPENAI_SHADES }
    else if p.contains("google") || p.contains("gemini") { &GOOGLE_SHADES }
    else if p.contains("deepseek") { &DEEPSEEK_SHADES }
    else if p.contains("xai") || p.contains("grok") { &XAI_SHADES }
    else if p.contains("meta") || p.contains("llama") { &META_SHADES }
    else if p.contains("cursor") { &CURSOR_SHADES }
    else if p.contains("minimax") || p.contains("mimo") { &MINIMAX_SHADES }
    else { &UNKNOWN_SHADES };

    let idx = rank.min(palette.len() - 1);
    let (r,g,b) = palette[idx];
    Color::Rgb(r,g,b)
}

pub fn get_model_color(model: &str) -> Color {
    get_provider_shade(get_provider_from_model(model), 0)
}

pub fn get_provider_from_model(model: &str) -> &'static str {
    let m = model.to_lowercase();
    if m.contains("claude") || m.contains("sonnet") || m.contains("opus") || m.contains("haiku") { "anthropic" }
    else if m.contains("gpt") || m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4") || m.contains("codex") { "openai" }
    else if m.contains("gemini") { "google" }
    else if m.contains("deepseek") { "deepseek" }
    else if m.contains("grok") { "xai" }
    else if m.contains("llama") { "meta" }
    else if m.contains("minimax") || m.contains("mimo") { "minimax" }
    else if m.contains("kimi") { "moonshot" }
    else if m.contains("qwen") { "qwen" }
    else { "unknown" }
}

const ANTHROPIC_SHADES: [(u8,u8,u8);7] = [(218,119,86),(223,136,107),(227,153,128),(232,170,149),(236,184,166),(239,197,183),(243,210,199)];
const OPENAI_SHADES: [(u8,u8,u8);7] = [(16,185,129),(18,208,145),(20,232,162),(41,236,172),(61,238,179),(97,241,193),(133,244,208)];
const GOOGLE_SHADES: [(u8,u8,u8);7] = [(59,130,246),(83,146,247),(108,161,248),(132,177,249),(153,190,250),(172,202,251),(190,214,252)];
const DEEPSEEK_SHADES: [(u8,u8,u8);7] = [(6,182,212),(7,203,237),(21,215,248),(45,219,249),(66,223,250),(85,226,250),(105,229,251)];
const XAI_SHADES: [(u8,u8,u8);7] = [(234,179,8),(247,192,21),(248,199,45),(249,205,70),(249,211,91),(250,216,110),(251,221,129)];
const META_SHADES: [(u8,u8,u8);7] = [(99,102,241),(122,125,243),(146,148,245),(169,171,247),(189,190,249),(207,208,251),(225,226,252)];
const CURSOR_SHADES: [(u8,u8,u8);7] = [(139,92,246),(154,114,247),(169,135,248),(184,156,250),(199,177,251),(215,199,252),(230,220,253)];
const MINIMAX_SHADES: [(u8,u8,u8);7] = [(255,107,53),(255,136,84),(255,156,109),(255,175,133),(255,192,156),(255,210,183),(255,228,210)];
const UNKNOWN_SHADES: [(u8,u8,u8);7] = [(136,136,136),(156,156,156),(176,176,176),(196,196,196),(212,212,212),(228,228,228),(244,244,244)];
