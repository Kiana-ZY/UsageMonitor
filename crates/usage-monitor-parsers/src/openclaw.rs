//! OpenClaw (formerly Clawdbot/Moltbot) session JSONL parser.
//!
//! Reads `~/.openclaw/agents/` transcript files.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;
use usage_monitor_core::{TokenBreakdown, UnifiedMessage};

use super::ParserError;

#[derive(Debug, Deserialize)]
struct OpenClawEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    role: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    usage: Option<OpenClawUsage>,
    timestamp: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenClawUsage {
    input: Option<i64>,
    output: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    total: Option<i64>,
}

/// Parse OpenClaw session files under `base_dir`.
pub fn parse_openclaw(base_dir: &Path) -> Result<Vec<UnifiedMessage>, ParserError> {
    let mut messages: Vec<UnifiedMessage> = Vec::new();

    if !base_dir.is_dir() {
        return Ok(messages);
    }

    for entry in walkdir::WalkDir::new(base_dir)
        .max_depth(4)
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
        let mut current_model = String::new();
        let mut current_provider = String::new();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let entry: OpenClawEntry = match serde_json::from_str(trimmed) {
                Ok(e) => e,
                Err(_) => continue,
            };

            match entry.entry_type.as_deref() {
                Some("model_change") => {
                    if let Some(ref m) = entry.model {
                        current_model = m.clone();
                    }
                    if let Some(ref p) = entry.provider {
                        current_provider = p.clone();
                    }
                }
                Some("message") => {
                    if entry.role.as_deref() != Some("assistant") {
                        continue;
                    }
                    let usage = match entry.usage {
                        Some(ref u) => u,
                        None => continue,
                    };

                    let model = entry
                        .model
                        .clone()
                        .or_else(|| {
                            if current_model.is_empty() {
                                None
                            } else {
                                Some(current_model.clone())
                            }
                        });

                    let model = match model {
                        Some(m) => m,
                        None => continue,
                    };

                    let provider = entry
                        .provider
                        .clone()
                        .unwrap_or_else(|| {
                            if current_provider.is_empty() {
                                "unknown".into()
                            } else {
                                current_provider.clone()
                            }
                        });

                    let ts = entry
                        .timestamp
                        .as_deref()
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.timestamp_millis())
                        .unwrap_or(fallback_ts);

                    messages.push(UnifiedMessage {
                        client: "openclaw".into(),
                        model_id: usage_monitor_core::normalize_model_id(&model),
                        provider_id: provider.clone(),
                        session_id: session_id.clone(),
                        timestamp: ts,
                        tokens: TokenBreakdown {
                            input: usage.input.unwrap_or(0).max(0),
                            output: usage.output.unwrap_or(0).max(0),
                            cache_read: usage.cache_read.unwrap_or(0).max(0),
                            cache_write: usage.cache_write.unwrap_or(0).max(0),
                            reasoning: 0,
                        },
                        cost: 0.0,
                        request_id: Some(format!("openclaw:{session_id}:{ts}")),
                        workspace: None,
                        data_source: "native".into(),
                    });

                    // Update tracked model/provider for subsequent messages
                    current_model = model;
                    current_provider = provider;
                }
                _ => {}
            }
        }
    }

    Ok(messages)
}
