use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use usage_monitor_core::TokenBreakdown;

// ── ModelPricing ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub cache_read_cost_per_token: f64,
    pub source: String,
}

// ── PricingEngine ─────────────────────────────────────────────

pub struct PricingEngine {
    pricing: Mutex<HashMap<String, ModelPricing>>,
    cache_dir: PathBuf,
}

impl PricingEngine {
    pub fn new(cc_switch_db: Option<&str>) -> Self {
        let home = dirs_next();
        let cache_dir = home.join(".usage-monitor").join("cache");
        std::fs::create_dir_all(&cache_dir).ok();

        let mut engine = Self {
            pricing: Mutex::new(HashMap::new()),
            cache_dir,
        };

        // 1. Load CC Switch pricing (highest priority for CC Switch users)
        if let Some(path) = cc_switch_db {
            engine.load_cc_switch(path);
        }

        // 2. Load cached LiteLLM data
        engine.load_cached();

        // 3. Manual seed for essential models (lowest priority, fills gaps)
        engine.seed_manual();

        // Background sync (spawn a thread)
        let cache = engine.cache_dir.clone();
        std::thread::spawn(move || {
            if let Ok(data) = fetch_litellm() {
                save_cache(&cache, &data);
            }
        });

        engine
    }

    fn load_cc_switch(&self, path: &str) {
        if let Ok(conn) = Connection::open(path) {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT model_id, input_cost_per_million, output_cost_per_million, cache_read_cost_per_million FROM model_pricing"
            ) {
                if let Ok(rows) = stmt.query_map([], |row| Ok((
                    row.get::<_,String>(0)?, row.get::<_,String>(1)?,
                    row.get::<_,String>(2)?, row.get::<_,String>(3)?,
                ))) {
                    let mut map = self.pricing.lock().unwrap();
                    for row in rows.flatten() {
                        let (id, inp, out, cache) = row;
                        let inp: f64 = inp.parse().unwrap_or(0.0);
                        let out: f64 = out.parse().unwrap_or(0.0);
                        let cr: f64 = cache.parse().unwrap_or(0.0);
                        map.entry(id).or_insert(ModelPricing {
                            input_cost_per_token: inp / 1_000_000.0,
                            output_cost_per_token: out / 1_000_000.0,
                            cache_read_cost_per_token: cr / 1_000_000.0,
                            source: "cc-switch".into(),
                        });
                    }
                }
            }
        }
    }

    fn load_cached(&self) {
        let cache_file = self.cache_dir.join("pricing-litellm.json");
        if let Ok(data) = std::fs::read_to_string(&cache_file) {
            if let Ok(prices) = serde_json::from_str::<HashMap<String,LiteLLMModel>>(&data) {
                let mut map = self.pricing.lock().unwrap();
                for (id, m) in prices {
                    let inp = m.input_cost_per_token.unwrap_or(0.0);
                    let out = m.output_cost_per_token.unwrap_or(0.0);
                    let cr = m.cache_read_input_token_cost.unwrap_or(0.0);
                    map.entry(id).or_insert(ModelPricing {
                        input_cost_per_token: inp, output_cost_per_token: out,
                        cache_read_cost_per_token: cr, source: "litellm".into(),
                    });
                }
            }
        }
    }

    fn seed_manual(&self) {
        // Comprehensive price list (per 1M tokens): (model, input, output, cache_read)
        let manual: &[(&str, f64, f64, f64)] = &[
            // Anthropic
            ("claude-opus-4-7", 15.00, 75.00, 1.50),
            ("claude-sonnet-4-6", 3.00, 15.00, 0.30),
            ("claude-haiku-4-5", 0.80, 4.00, 0.08),
            ("claude-3.5-sonnet", 3.00, 15.00, 0.30),
            ("claude-3.5-haiku", 0.80, 4.00, 0.08),
            // OpenAI
            ("gpt-5.4", 1.75, 14.00, 1.75),
            ("gpt-5.3", 1.75, 14.00, 0.175),
            ("gpt-5.2", 1.75, 14.00, 0.175),
            ("gpt-5.1", 2.50, 10.00, 1.25),
            ("gpt-4o", 2.50, 10.00, 1.25),
            ("gpt-4o-mini", 0.15, 0.60, 0.075),
            ("o4-mini", 1.10, 4.40, 0.275),
            ("o3", 10.00, 40.00, 2.50),
            ("o1", 15.00, 60.00, 7.50),
            ("gpt-5.3-codex", 1.75, 14.00, 0.175),
            ("gpt-5.3-codex-spark", 1.75, 14.00, 0.175),
            // Google
            ("gemini-2.5-pro", 1.25, 10.00, 0.25),
            ("gemini-2.5-flash", 0.15, 0.60, 0.02),
            ("gemini-2.0-flash", 0.10, 0.40, 0.025),
            ("gemini-2.5-pro-preview", 1.25, 10.00, 0.25),
            ("gemini-2.5-flash-lite", 0.10, 0.40, 0.01),
            // DeepSeek
            ("deepseek-v4-pro", 0.20, 3.36, 0.14),
            ("deepseek-v4-flash", 0.14, 3.36, 0.14),
            ("deepseek-v3", 0.27, 1.10, 0.07),
            ("deepseek-r1", 0.55, 2.19, 0.14),
            // MiniMax
            ("minimax-m2-7", 0.20, 1.10, 0.20),
            ("minimax-m2-5", 0.20, 1.10, 0.20),
            ("minimax-m1", 0.40, 1.60, 0.40),
            ("mimo-v2-5", 0.20, 1.10, 0.20),
            ("mimo-v2-5-pro", 0.20, 1.10, 0.20),
            ("mimo-v2-7", 0.20, 1.10, 0.20),
            // xAI
            ("grok-4", 2.00, 8.00, 0.50),
            ("grok-3", 3.00, 15.00, 0.50),
            // Meta
            ("llama-4-maverick", 0.20, 0.90, 0.05),
            ("llama-4-scout", 0.12, 0.50, 0.03),
            // Cursor
            ("composer-1", 1.25, 10.00, 0.125),
            ("composer-1.5", 3.50, 17.50, 0.35),
            ("composer-2", 0.50, 2.50, 0.20),
            ("composer-2-fast", 1.50, 7.50, 0.35),
            // Kimi / Moonshot
            ("kimi-for-coding", 0.40, 1.60, 0.10),
            // Qwen
            ("qwen-coder-plus", 0.40, 1.60, 0.10),
            ("qwen3-coder", 0.20, 0.80, 0.05),
        ];

        let mut map = self.pricing.lock().unwrap();
        for (model, inp, out, cache) in manual {
            let key = model.to_string();
            map.entry(key).or_insert(ModelPricing {
                input_cost_per_token: inp / 1_000_000.0,
                output_cost_per_token: out / 1_000_000.0,
                cache_read_cost_per_token: cache / 1_000_000.0,
                source: "manual".into(),
            });
        }
    }

    // ── Lookup with fuzzy matching ──────────────────────────

    pub fn lookup(&self, model_id: &str) -> Option<ModelPricing> {
        let map = self.pricing.lock().unwrap();

        // Step 1: Exact match
        if let Some(p) = map.get(model_id) { return Some(p.clone()); }

        let lower = model_id.to_lowercase();

        // Step 2: Case-insensitive exact match
        if let Some(p) = map.get(&lower) { return Some(p.clone()); }

        // Step 3: Normalized (dots→dashes, dashes→dots)
        let norm_dot = lower.replace('-', ".");
        if let Some(p) = map.get(&norm_dot) { return Some(p.clone()); }
        let norm_dash = lower.replace('.', "-");
        if let Some(p) = map.get(&norm_dash) { return Some(p.clone()); }

        // Step 4: Strip provider prefix (openai/gpt-5.4 → gpt-5.4)
        if let Some(idx) = lower.find('/') {
            let stripped = &lower[idx+1..];
            if let Some(p) = map.get(stripped) { return Some(p.clone()); }
        }

        // Step 5: Strip date suffixes (gpt-5.3-2025-08-01 → gpt-5.3)
        let stripped = strip_date_suffix(&lower);
        if stripped != lower {
            if let Some(p) = map.get(stripped) { return Some(p.clone()); }
            // also try normalized
            let nd = stripped.replace('.', "-");
            if nd != *stripped {
                if let Some(p) = map.get(&nd) { return Some(p.clone()); }
            }
        }

        // Step 6: Strip tier suffixes (high, medium, xhigh, thinking, fast, codex, spark)
        for suffix in &["-high","-medium","-xhigh","-thinking","-fast","-codex","-spark","-lite","-preview"] {
            if let Some(base) = lower.strip_suffix(suffix) {
                if let Some(p) = map.get(base) { return Some(p.clone()); }
                // also try dots→dashes
                let nd = base.replace('.', "-");
                if let Some(p) = map.get(&nd) { return Some(p.clone()); }
            }
        }

        // Step 7: Substring/fuzzy — longest key that is a prefix of model_id
        let mut best: Option<(usize, &ModelPricing)> = None;
        for (k, v) in map.iter() {
            if lower.starts_with(&k.to_lowercase()) && k.len() > best.map(|(l,_)| l).unwrap_or(0) {
                best = Some((k.len(), v));
            }
        }
        best.map(|(_, v)| v.clone())
    }

    pub fn calculate_cost(&self, model_id: &str, tokens: &TokenBreakdown) -> f64 {
        match self.lookup(model_id) {
            Some(p) => {
                tokens.input as f64 * p.input_cost_per_token
                    + tokens.output as f64 * p.output_cost_per_token
                    + tokens.cache_read as f64 * p.cache_read_cost_per_token
            }
            None => 0.0,
        }
    }
}

// ── LiteLLM fetch ───────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct LiteLLMModel {
    input_cost_per_token: Option<f64>,
    output_cost_per_token: Option<f64>,
    cache_read_input_token_cost: Option<f64>,
    #[serde(rename = "cache_creation_input_token_cost")]
    _cache_creation: Option<f64>,
}

fn fetch_litellm() -> Result<HashMap<String, LiteLLMModel>, String> {
    let url = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| format!("HTTP error: {}", e))?
        .into_string()
        .map_err(|e| format!("Read error: {}", e))?;

    let data: HashMap<String, serde_json::Value> =
        serde_json::from_str(&resp).map_err(|e| format!("Parse error: {}", e))?;

    let mut models = HashMap::new();
    for (key, val) in data {
        // Skip metadata entries & subscription providers
        if key.starts_with("sample_") || key == "lm_studio" { continue; }
        if key.starts_with("github_copilot/") { continue; }

        if let Ok(model) = serde_json::from_value::<LiteLLMModel>(val.clone()) {
            if model.input_cost_per_token.is_some() || model.output_cost_per_token.is_some() {
                models.insert(key, model);
            }
        }
    }
    Ok(models)
}

fn save_cache(dir: &PathBuf, data: &HashMap<String, LiteLLMModel>) {
    let tmp = dir.join("pricing-litellm.json.tmp");
    let target = dir.join("pricing-litellm.json");
    if let Ok(json) = serde_json::to_string(data) {
        std::fs::write(&tmp, json).ok();
        std::fs::rename(&tmp, &target).ok();
    }
}

// ── Helpers ──────────────────────────────────────────────────

fn strip_date_suffix(s: &str) -> &str {
    if s.len() > 11 {
        let tail = &s[s.len()-11..];
        if tail.starts_with('-')
            && tail[1..5].chars().all(|c| c.is_ascii_digit())
            && tail.chars().nth(5) == Some('-')
            && tail[6..8].chars().all(|c| c.is_ascii_digit())
            && tail.chars().nth(8) == Some('-')
            && tail[9..11].chars().all(|c| c.is_ascii_digit())
        { return &s[..s.len()-11]; }
    }
    s
}

fn dirs_next() -> PathBuf {
    #[cfg(target_os = "windows")]
    { std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_default() }
    #[cfg(not(target_os = "windows"))]
    { std::env::var("HOME").map(PathBuf::from).unwrap_or_default() }
}
