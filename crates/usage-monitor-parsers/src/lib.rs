//! Native session file parsers for AI coding tools.
//!
//! Each module parses one tool's session format into `UnifiedMessage` records.
//! All parsers are independent and can be called individually or via `parse_all()`.

pub mod claude_code;
pub mod codex;
pub mod gemini;
pub mod hermes;
pub mod kimi;
pub mod openclaw;
pub mod pi;

use std::collections::HashSet;
use std::path::Path;

use usage_monitor_core::UnifiedMessage;

/// Error type shared across all parsers.
#[derive(Debug, thiserror::Error)]
pub enum ParserError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parse error: {0}")]
    Json(String),
}

/// Run all available parsers, deduplicate, and return sorted results.
pub fn parse_all(
    claude_code_dir: Option<&Path>,
    codex_dir: Option<&Path>,
    kimi_dir: Option<&Path>,
    gemini_dir: Option<&Path>,
    pi_dir: Option<&Path>,
    openclaw_dir: Option<&Path>,
    hermes_db: Option<&Path>,
) -> Vec<UnifiedMessage> {
    let mut all = Vec::new();

    let mut push = |result: Result<Vec<UnifiedMessage>, ParserError>| {
        if let Ok(msgs) = result {
            all.extend(msgs);
        }
    };

    if let Some(dir) = claude_code_dir {
        push(claude_code::parse_claude_code(dir));
    }
    if let Some(dir) = codex_dir {
        push(codex::parse_codex(dir));
    }
    if let Some(dir) = kimi_dir {
        push(kimi::parse_kimi(dir));
    }
    if let Some(dir) = gemini_dir {
        push(gemini::parse_gemini(dir));
    }
    if let Some(dir) = pi_dir {
        push(pi::parse_pi(dir));
    }
    if let Some(dir) = openclaw_dir {
        push(openclaw::parse_openclaw(dir));
    }
    if let Some(path) = hermes_db {
        push(hermes::parse_hermes(path));
    }

    // Dedup by (client, request_id, timestamp)
    dedup_messages(&mut all);
    all.sort_by(|a, b| {
        a.timestamp
            .cmp(&b.timestamp)
            .then_with(|| a.client.cmp(&b.client))
    });
    all
}

fn dedup_messages(messages: &mut Vec<UnifiedMessage>) {
    let mut seen = HashSet::new();
    messages.retain(|m| {
        let key = (
            m.client.clone(),
            m.request_id.clone(),
            m.timestamp,
        );
        seen.insert(key)
    });
}
