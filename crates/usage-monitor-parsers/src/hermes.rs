//! Hermes session parser.
//!
//! Reads Hermes SQLite database — queries the `sessions` table.

use std::path::Path;

use usage_monitor_core::{TokenBreakdown, UnifiedMessage};

use super::ParserError;

/// Parse Hermes SQLite database at `db_path`.
pub fn parse_hermes(db_path: &Path) -> Result<Vec<UnifiedMessage>, ParserError> {
    let mut messages: Vec<UnifiedMessage> = Vec::new();

    if !db_path.exists() {
        return Ok(messages);
    }

    // Try to open as SQLite
    let conn = match rusqlite::Connection::open(db_path) {
        Ok(c) => c,
        Err(_) => return Ok(messages),
    };

    let mut stmt = match conn.prepare(
        "SELECT id, model, billing_provider, started_at, message_count,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                reasoning_tokens, estimated_cost_usd, actual_cost_usd
         FROM sessions
         WHERE model IS NOT NULL AND model != ''
           AND (input_tokens > 0 OR output_tokens > 0
                OR cache_read_tokens > 0 OR estimated_cost_usd > 0)"
    ) {
        Ok(s) => s,
        Err(_) => return Ok(messages),
    };

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,               // id
            row.get::<_, String>(1)?,               // model
            row.get::<_, Option<String>>(2)?,        // billing_provider
            row.get::<_, Option<i64>>(3)?,           // started_at
            row.get::<_, i32>(4)?,                   // message_count
            row.get::<_, i64>(5)?,                   // input_tokens
            row.get::<_, i64>(6)?,                   // output_tokens
            row.get::<_, i64>(7)?,                   // cache_read_tokens
            row.get::<_, i64>(8)?,                   // cache_write_tokens
            row.get::<_, i64>(9)?,                   // reasoning_tokens
            row.get::<_, Option<f64>>(10)?,          // estimated_cost_usd
            row.get::<_, Option<f64>>(11)?,          // actual_cost_usd
        ))
    });

    match rows {
        Ok(iter) => {
            for row in iter.flatten() {
                let (
                    id,
                    model,
                    billing_provider,
                    started_at,
                    _msg_count,
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    reasoning_tokens,
                    estimated_cost_usd,
                    actual_cost_usd,
                ) = row;

                let provider = billing_provider
                    .filter(|p| !p.is_empty())
                    .unwrap_or_else(|| "hermes".into());

                let timestamp = started_at.map_or(0, |ts| {
                    if ts > 1_000_000_000_000 { ts } else { ts * 1000 }
                });

                let cost = actual_cost_usd
                    .or(estimated_cost_usd)
                    .unwrap_or(0.0)
                    .max(0.0);

                messages.push(UnifiedMessage {
                    client: "hermes".into(),
                    model_id: usage_monitor_core::normalize_model_id(&model),
                    provider_id: provider,
                    session_id: id.clone(),
                    timestamp,
                    tokens: TokenBreakdown {
                        input: input_tokens.max(0),
                        output: output_tokens.max(0),
                        cache_read: cache_read_tokens.max(0),
                        cache_write: cache_write_tokens.max(0),
                        reasoning: reasoning_tokens.max(0),
                    },
                    cost,
                    request_id: Some(format!("hermes:{id}")),
                    workspace: None,
                    data_source: "native".into(),
                });
            }
        }
        Err(_) => {}
    }

    Ok(messages)
}
