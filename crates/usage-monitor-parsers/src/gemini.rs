//! Gemini CLI session parser.
//!
//! Reads `~/.gemini/tmp/` — both legacy `session-*.json` and JSONL formats.

use std::fs;
use std::path::Path;

use serde::Deserialize;
use usage_monitor_core::{TokenBreakdown, UnifiedMessage};

use super::ParserError;

#[derive(Debug, Deserialize)]
struct GeminiSession {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    messages: Option<Vec<GeminiMsg>>,
}

#[derive(Debug, Deserialize)]
struct GeminiMsg {
    #[serde(rename = "type")]
    msg_type: Option<String>,
    model: Option<String>,
    usage: Option<GeminiUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct GeminiUsage {
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cached_tokens: Option<i64>,
    thoughts_tokens: Option<i64>,
}

/// Parse Gemini session files under `base_dir`.
pub fn parse_gemini(base_dir: &Path) -> Result<Vec<UnifiedMessage>, ParserError> {
    let mut messages: Vec<UnifiedMessage> = Vec::new();

    if !base_dir.is_dir() {
        return Ok(messages);
    }

    for entry in walkdir::WalkDir::new(base_dir)
        .max_depth(4)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let p = e.path();
            let ext = p.extension().map_or("".into(), |e| e.to_string_lossy().to_string());
            ext == "json" || ext == "jsonl"
        })
    {
        let path = entry.path();

        // Try structured JSON format first
        if path.extension().map_or(false, |e| e == "json") {
            if let Ok(data) = fs::read_to_string(path) {
                if let Ok(session) = serde_json::from_str::<GeminiSession>(&data) {
                    let sid = session
                        .session_id
                        .unwrap_or_else(|| path.file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default());
                    if let Some(msgs) = session.messages {
                        for msg in msgs {
                            if msg.msg_type.as_deref() != Some("gemini") {
                                continue;
                            }
                            if let Some(usage) = msg.usage {
                                let (input, cache_read) = usage_monitor_core::subtract_cached_overlap(
                                    usage.input_tokens.unwrap_or(0),
                                    usage.cached_tokens.unwrap_or(0),
                                );
                                let model = msg.model.unwrap_or_else(|| "gemini".into());
                                let normalized = usage_monitor_core::normalize_model_id(&model);
                                let fallback_ts = entry
                                    .metadata().ok()
                                    .and_then(|m| m.modified().ok())
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map(|d| d.as_millis() as i64)
                                    .unwrap_or(0);

                                messages.push(UnifiedMessage {
                                    client: "gemini".into(),
                                    model_id: normalized,
                                    provider_id: "google".into(),
                                    session_id: sid.clone(),
                                    timestamp: fallback_ts,
                                    tokens: TokenBreakdown {
                                        input: input.max(0),
                                        output: usage.output_tokens.unwrap_or(0).max(0),
                                        cache_read: cache_read.max(0),
                                        cache_write: 0,
                                        reasoning: usage.thoughts_tokens.unwrap_or(0).max(0),
                                    },
                                    cost: 0.0,
                                    request_id: Some(format!("gemini:{sid}:{fallback_ts}")),
                                    workspace: None,
                                    data_source: "native".into(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(messages)
}
