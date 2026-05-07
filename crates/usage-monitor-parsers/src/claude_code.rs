//! Claude Code session JSONL parser.
//!
//! Reads `~/.claude/projects/<workspace>/<uuid>.jsonl` files.
//! Handles streaming dedup: when the same `parentUuid` appears multiple times,
//! each token field takes the maximum value seen (streaming partial writes).

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;
use usage_monitor_core::{TokenBreakdown, UnifiedMessage};

use super::ParserError;

#[derive(Debug, Deserialize)]
struct ClaudeLine {
    #[serde(rename = "parentUuid")]
    parent_uuid: Option<String>,
    #[serde(rename = "isSidechain")]
    is_sidechain: Option<bool>,
    message: Option<ClaudeMessage>,
}

#[derive(Debug, Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    role: Option<String>,
    model: Option<String>,
    usage: Option<ClaudeUsage>,
}

#[derive(Debug, Default, Deserialize)]
struct ClaudeUsage {
    input_tokens: i64,
    output_tokens: i64,
    cache_read_input_tokens: i64,
    cache_creation_input_tokens: i64,
}

/// Parse Claude Code session files under `base_dir`.
pub fn parse_claude_code(base_dir: &Path) -> Result<Vec<UnifiedMessage>, ParserError> {
    let mut messages: Vec<UnifiedMessage> = Vec::new();

    if !base_dir.is_dir() {
        return Ok(messages);
    }

    for entry in walkdir::WalkDir::new(base_dir)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "jsonl"))
        .filter(|e| !e.path().to_string_lossy().contains("subagents"))
    {
        let file = match fs::File::open(entry.path()) {
            Ok(f) => f,
            Err(_) => continue,
        };
        let reader = BufReader::new(file);

        let session_id = entry
            .path()
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut dedup: HashMap<String, (ClaudeUsage, String)> = HashMap::new();
        let fallback_ts = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let entry: ClaudeLine = match serde_json::from_str(trimmed) {
                Ok(e) => e,
                Err(_) => continue,
            };

            if entry.is_sidechain == Some(true) {
                continue;
            }

            let msg = match entry.message {
                Some(m) => m,
                None => continue,
            };

            if msg.role.as_deref() != Some("assistant") {
                continue;
            }

            let usage = match msg.usage {
                Some(u) => u,
                None => continue,
            };

            let key = entry.parent_uuid.unwrap_or_else(|| {
                msg.id.clone().unwrap_or_default()
            });

            let model = msg.model.clone().unwrap_or_else(|| "unknown".into());

            match dedup.get_mut(&key) {
                Some((existing, _existing_model)) => {
                    existing.input_tokens = existing.input_tokens.max(usage.input_tokens);
                    existing.output_tokens = existing.output_tokens.max(usage.output_tokens);
                    existing.cache_read_input_tokens = existing
                        .cache_read_input_tokens
                        .max(usage.cache_read_input_tokens);
                    existing.cache_creation_input_tokens = existing
                        .cache_creation_input_tokens
                        .max(usage.cache_creation_input_tokens);
                }
                None => {
                    dedup.insert(key, (ClaudeUsage {
                        input_tokens: usage.input_tokens.max(0),
                        output_tokens: usage.output_tokens.max(0),
                        cache_read_input_tokens: usage.cache_read_input_tokens.max(0),
                        cache_creation_input_tokens: usage.cache_creation_input_tokens.max(0),
                    }, model));
                }
            }
        }

        for (dedup_key, (usage, model)) in dedup {
            let normalized = usage_monitor_core::normalize_model_id(&model);
            let provider = provider_from_model(&normalized);
            messages.push(UnifiedMessage {
                client: "claude".into(),
                model_id: normalized,
                provider_id: provider,
                session_id: session_id.clone(),
                timestamp: fallback_ts,
                tokens: TokenBreakdown {
                    input: usage.input_tokens,
                    output: usage.output_tokens,
                    cache_read: usage.cache_read_input_tokens,
                    cache_write: usage.cache_creation_input_tokens,
                    reasoning: 0,
                },
                cost: 0.0,
                request_id: Some(dedup_key),
                workspace: None,
                data_source: "native".into(),
            });
        }
    }

    messages.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    Ok(messages)
}

fn provider_from_model(model: &str) -> String {
    let m = model.to_lowercase();
    if m.contains("claude") || m.contains("sonnet") || m.contains("haiku") || m.contains("opus") {
        "anthropic".into()
    } else if m.contains("gpt") || m.contains("o1") || m.contains("o3") || m.contains("o4") {
        "openai".into()
    } else if m.contains("gemini") {
        "google".into()
    } else if m.contains("deepseek") {
        "deepseek".into()
    } else if m.contains("minimax") || m.contains("mimo") {
        "minimax".into()
    } else {
        "unknown".into()
    }
}
