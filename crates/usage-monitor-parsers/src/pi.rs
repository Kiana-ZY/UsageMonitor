//! Pi session JSONL parser.
//!
//! Reads `~/.omp/agent/sessions/` JSONL files.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;
use usage_monitor_core::{TokenBreakdown, UnifiedMessage};

use super::ParserError;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct PiEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    role: Option<String>,
    model: Option<String>,
    provider: Option<String>,
    usage: Option<PiUsage>,
    timestamp: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PiUsage {
    input: Option<i64>,
    output: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
}

/// Parse Pi session files under `base_dir`.
pub fn parse_pi(base_dir: &Path) -> Result<Vec<UnifiedMessage>, ParserError> {
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

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let entry: PiEntry = match serde_json::from_str(trimmed) {
                Ok(e) => e,
                Err(_) => continue,
            };

            if entry.role.as_deref() != Some("assistant") {
                continue;
            }

            let usage = match entry.usage {
                Some(u) => u,
                None => continue,
            };

            let model = match &entry.model {
                Some(m) if !m.is_empty() => m.clone(),
                _ => continue,
            };
            let provider = entry
                .provider
                .clone()
                .unwrap_or_else(|| "unknown".into());

            let ts = entry
                .timestamp
                .as_deref()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp_millis())
                .unwrap_or(fallback_ts);

            messages.push(UnifiedMessage {
                client: "pi".into(),
                model_id: usage_monitor_core::normalize_model_id(&model),
                provider_id: provider,
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
                request_id: Some(format!("pi:{session_id}:{ts}")),
                workspace: None,
                data_source: "native".into(),
            });
        }
    }

    Ok(messages)
}
