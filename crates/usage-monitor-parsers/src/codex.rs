//! Codex CLI session JSONL parser.
//!
//! Reads Codex session files. Handles token_count events with
//! delta calculation from cumulative totals.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;
use usage_monitor_core::{TokenBreakdown, UnifiedMessage};

use super::ParserError;

#[derive(Debug, Deserialize)]
struct CodexEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    timestamp: Option<String>,
    payload: Option<serde_json::Value>,
}

/// Parse Codex session files under `base_dir`.
pub fn parse_codex(base_dir: &Path) -> Result<Vec<UnifiedMessage>, ParserError> {
    let mut messages: Vec<UnifiedMessage> = Vec::new();

    if !base_dir.is_dir() {
        return Ok(messages);
    }

    for entry in walkdir::WalkDir::new(base_dir)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "jsonl"))
    {
        let file = match fs::File::open(entry.path()) {
            Ok(f) => f,
            Err(_) => continue,
        };

        let fallback_ts = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let session_id = entry
            .path()
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let reader = BufReader::new(file);
        let mut prev_total: Option<(i64, i64, i64)> = None;
        let mut current_model = String::from("unknown");

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let entry: CodexEntry = match serde_json::from_str(trimmed) {
                Ok(e) => e,
                Err(_) => continue,
            };

            let payload = match entry.payload {
                Some(p) => p,
                None => continue,
            };

            match entry.entry_type.as_deref() {
                Some("session_meta") => {
                    if let Some(model) = payload.get("model").and_then(|v| v.as_str()) {
                        current_model = model.to_string();
                    }
                }
                Some("turn_context") => {
                    if let Some(model) = payload.get("model").and_then(|v| v.as_str()) {
                        current_model = model.to_string();
                    }
                }
                Some("event_msg") => {
                    if payload.get("type").and_then(|v| v.as_str()) != Some("token_count") {
                        continue;
                    }
                    let info = match payload.get("info") {
                        Some(i) => i,
                        None => continue,
                    };
                    let total = info.get("total_token_usage");
                    let last = info.get("last_token_usage");

                    let (input, output, cache_read) = if let Some(last) = last {
                        let inp = last.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let out = last.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cache = last
                            .get("cached_input_tokens")
                            .or_else(|| last.get("cache_read_input_tokens"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);

                        if let Some(total) = total {
                            let tot_inp =
                                total.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                            let tot_out =
                                total.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                            let tot_cache = total
                                .get("cached_input_tokens")
                                .or_else(|| total.get("cache_read_input_tokens"))
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0);
                            prev_total = Some((tot_inp, tot_out, tot_cache));
                        }
                        (inp.max(0), out.max(0), cache.max(0))
                    } else if let Some(total) = total {
                        let inp =
                            total.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let out =
                            total.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                        let cache = total
                            .get("cached_input_tokens")
                            .or_else(|| total.get("cache_read_input_tokens"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);

                        let (delta_inp, delta_out, delta_cache) = if let Some((pi, po, pc)) =
                            prev_total
                        {
                            (inp.saturating_sub(pi), out.saturating_sub(po), cache.saturating_sub(pc))
                        } else {
                            (inp, out, cache)
                        };
                        prev_total = Some((inp, out, cache));
                        (delta_inp.max(0), delta_out.max(0), delta_cache.max(0))
                    } else {
                        continue;
                    };

                    if input == 0 && output == 0 && cache_read == 0 {
                        continue;
                    }

                    let ts = parse_codex_ts(&entry.timestamp, fallback_ts);

                    let normalized = usage_monitor_core::normalize_model_id(&current_model);
                    let provider = "unknown".to_string(); // provider inferred later
                    messages.push(UnifiedMessage {
                        client: "codex".into(),
                        model_id: normalized,
                        provider_id: provider,
                        session_id: session_id.clone(),
                        timestamp: ts,
                        tokens: TokenBreakdown {
                            input,
                            output,
                            cache_read,
                            cache_write: 0,
                            reasoning: 0,
                        },
                        cost: 0.0,
                        request_id: Some(format!("codex:{session_id}:{ts}")),
                        workspace: None,
                        data_source: "native".into(),
                    });
                }
                _ => {}
            }
        }
    }

    Ok(messages)
}

fn parse_codex_ts(ts: &Option<String>, fallback: i64) -> i64 {
    if let Some(s) = ts {
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
            return dt.timestamp_millis();
        }
    }
    fallback
}
