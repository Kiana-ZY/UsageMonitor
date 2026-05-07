//! Kimi Code wire.jsonl parser.
//!
//! Reads `~/.kimi/sessions/<group>/<uuid>/wire.jsonl`.
//! Only processes `StatusUpdate` messages with `token_usage`.
//! Reasoning tokens are not exposed by the Kimi wire protocol.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;
use usage_monitor_core::{TokenBreakdown, UnifiedMessage};

use super::ParserError;

#[derive(Debug, Deserialize)]
struct WireLine {
    timestamp: Option<f64>,
    message: Option<WireMessage>,
    #[serde(rename = "type")]
    wire_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireMessage {
    msg_type: Option<String>,
    token_usage: Option<KimiTokenUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct KimiTokenUsage {
    input_other: i64,
    output: i64,
    input_cache_read: i64,
    input_cache_creation: i64,
}

/// Parse Kimi Code wire files under `base_dir`.
pub fn parse_kimi(base_dir: &Path) -> Result<Vec<UnifiedMessage>, ParserError> {
    let mut messages: Vec<UnifiedMessage> = Vec::new();

    if !base_dir.is_dir() {
        return Ok(messages);
    }

    for entry in walkdir::WalkDir::new(base_dir)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().file_name().map_or(false, |n| n == "wire.jsonl"))
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
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
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

            let wire: WireLine = match serde_json::from_str(trimmed) {
                Ok(w) => w,
                Err(_) => continue,
            };

            if wire.wire_type.as_deref() == Some("metadata") {
                continue;
            }

            let msg = match wire.message {
                Some(m) => m,
                None => continue,
            };

            if msg.msg_type.as_deref() != Some("StatusUpdate") {
                continue;
            }

            let usage = match msg.token_usage {
                Some(u) => u,
                None => continue,
            };

            if usage.input_other <= 0 && usage.output <= 0
                && usage.input_cache_read <= 0 && usage.input_cache_creation <= 0
            {
                continue;
            }

            let ts = if let Some(t) = wire.timestamp {
                (t * 1000.0) as i64
            } else {
                fallback_ts
            };

            messages.push(UnifiedMessage {
                client: "kimi".into(),
                model_id: "kimi-for-coding".into(),
                provider_id: "moonshot".into(),
                session_id: session_id.clone(),
                timestamp: ts,
                tokens: TokenBreakdown {
                    input: usage.input_other.max(0),
                    output: usage.output.max(0),
                    cache_read: usage.input_cache_read.max(0),
                    cache_write: usage.input_cache_creation.max(0),
                    reasoning: 0, // Kimi wire does not expose reasoning tokens
                },
                cost: 0.0,
                request_id: Some(format!("kimi:{session_id}:{ts}")),
                workspace: None,
                data_source: "native".into(),
            });
        }
    }

    Ok(messages)
}
