//! Model pricing engine.
//!
//! Pricing sources (in priority order):
//! 1. CC Switch `model_pricing` table
//! 2. Manual fallback for known models

use std::collections::HashMap;

use rusqlite::Connection;
use usage_monitor_core::TokenBreakdown;

#[derive(Debug, Clone)]
pub struct ModelPricing {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    pub cache_read_cost_per_token: f64,
    pub source: String,
}

pub struct PricingEngine {
    pricing: HashMap<String, ModelPricing>,
}

impl PricingEngine {
    /// Create engine, loading CC Switch pricing if DB exists.
    pub fn new(cc_switch_db: Option<&str>) -> Self {
        let mut pricing = HashMap::new();

        // Load CC Switch pricing
        if let Some(path) = cc_switch_db {
            if let Ok(conn) = Connection::open(path) {
                if let Ok(mut stmt) = conn.prepare(
                    "SELECT model_id, input_cost_per_million, output_cost_per_million,
                            cache_read_cost_per_million
                     FROM model_pricing",
                ) {
                    if let Ok(rows) = stmt.query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    }) {
                        for row in rows.flatten() {
                            let (id, inp, out, cache) = row;
                            let inp_f: f64 = inp.parse().unwrap_or(0.0);
                            let out_f: f64 = out.parse().unwrap_or(0.0);
                            let cache_f: f64 = cache.parse().unwrap_or(0.0);
                            pricing.insert(
                                id.clone(),
                                ModelPricing {
                                    input_cost_per_token: inp_f / 1_000_000.0,
                                    output_cost_per_token: out_f / 1_000_000.0,
                                    cache_read_cost_per_token: cache_f / 1_000_000.0,
                                    source: "cc-switch".into(),
                                },
                            );
                        }
                    }
                }
            }
        }

        // Manual fallback for well-known models
        // Prices per 1M tokens → converted to per-token
        let manual: &[(&str, f64, f64, f64)] = &[
            ("deepseek-v4-pro", 0.20, 3.36, 0.14),     // DeepSeek V4 Pro
            ("deepseek-v4-flash", 0.14, 3.36, 0.14),
            ("minimax-m2-5", 0.20, 1.10, 0.20),
            ("minimax-m2-7", 0.20, 1.10, 0.20),
            ("gpt-5.4", 1.75, 14.00, 1.75),
            ("claude-sonnet-4-6", 3.00, 15.00, 0.30),
            ("claude-haiku-4-5", 0.80, 4.00, 0.08),
            ("claude-opus-4-7", 15.00, 75.00, 1.50),
            ("gemini-2.5-pro", 1.25, 10.00, 0.25),
            ("gemini-2.5-flash", 0.15, 0.60, 0.02),
        ];

        for (model, inp, out, cache) in manual {
            let key = model.to_string();
            if !pricing.contains_key(&key) {
                pricing.insert(
                    key,
                    ModelPricing {
                        input_cost_per_token: inp / 1_000_000.0,
                        output_cost_per_token: out / 1_000_000.0,
                        cache_read_cost_per_token: cache / 1_000_000.0,
                        source: "manual".into(),
                    },
                );
            }
        }

        Self { pricing }
    }

    /// Look up pricing for a model id. Returns None if unknown.
    pub fn lookup(&self, model_id: &str) -> Option<&ModelPricing> {
        // Exact match first
        if let Some(p) = self.pricing.get(model_id) {
            return Some(p);
        }
        // Try prefix match (strip provider prefix)
        if let Some(idx) = model_id.find('/') {
            if let Some(p) = self.pricing.get(&model_id[idx + 1..]) {
                return Some(p);
            }
        }
        None
    }

    /// Calculate cost for a TokenBreakdown given a model.
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
