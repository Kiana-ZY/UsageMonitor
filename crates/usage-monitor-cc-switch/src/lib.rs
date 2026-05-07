//! CC Switch SQLite adapter.
//!
//! Reads `~/.cc-switch/cc-switch.db` `proxy_request_logs` table
//! and converts records to `UnifiedMessage`.

use std::path::PathBuf;

use rusqlite::Connection;
use usage_monitor_core::{DataSource, DataSourceError, TokenBreakdown, UnifiedMessage};

/// CC Switch adapter. Zero-config — auto-discovers `~/.cc-switch/cc-switch.db`.
pub struct CcSwitchAdapter {
    db_path: PathBuf,
}

impl CcSwitchAdapter {
    /// Create a new adapter. `db_path` is typically `~/.cc-switch/cc-switch.db`.
    pub fn new(db_path: PathBuf) -> Self {
        Self { db_path }
    }

    /// Default path: `$HOME/.cc-switch/cc-switch.db`
    pub fn default_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        #[cfg(not(target_os = "windows"))]
        let home = std::env::var("HOME").unwrap_or_default();

        PathBuf::from(home).join(".cc-switch").join("cc-switch.db")
    }

    fn read_messages(&self) -> Result<Vec<UnifiedMessage>, DataSourceError> {
        let conn = Connection::open(&self.db_path)
            .map_err(|e| DataSourceError::NotAvailable(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT request_id, provider_id, app_type, model,
                        input_tokens, output_tokens, cache_read_tokens,
                        cache_creation_tokens, total_cost_usd, session_id,
                        created_at, data_source
                 FROM proxy_request_logs
                 ORDER BY created_at",
            )
            .map_err(|e| DataSourceError::ParseError(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,    // request_id
                    row.get::<_, String>(1)?,    // provider_id
                    row.get::<_, String>(2)?,    // app_type
                    row.get::<_, String>(3)?,    // model
                    row.get::<_, i64>(4)?,       // input_tokens
                    row.get::<_, i64>(5)?,       // output_tokens
                    row.get::<_, i64>(6)?,       // cache_read_tokens
                    row.get::<_, i64>(7)?,       // cache_creation_tokens
                    row.get::<_, String>(8)?,    // total_cost_usd
                    row.get::<_, Option<String>>(9)?, // session_id
                    row.get::<_, i64>(10)?,      // created_at
                    row.get::<_, String>(11)?,   // data_source
                ))
            })
            .map_err(|e| DataSourceError::ParseError(e.to_string()))?;

        let mut messages = Vec::new();
        for row in rows.flatten() {
            let (
                request_id,
                provider_id,
                app_type,
                model,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                total_cost_usd,
                session_id,
                created_at,
                data_source,
            ) = row;

            let cost: f64 = total_cost_usd.parse().unwrap_or(0.0);

            let client = if app_type == "claude" { "claude" } else { "codex" };

            let normalized = usage_monitor_core::normalize_model_id(&model);

            messages.push(UnifiedMessage {
                client: client.into(),
                model_id: normalized,
                provider_id,
                session_id: session_id.unwrap_or_default(),
                timestamp: created_at.max(0),
                tokens: TokenBreakdown {
                    input: input_tokens.max(0),
                    output: output_tokens.max(0),
                    cache_read: cache_read_tokens.max(0),
                    cache_write: cache_creation_tokens.max(0),
                    reasoning: 0,
                },
                cost: cost.max(0.0),
                request_id: Some(request_id),
                workspace: None,
                data_source,
            });
        }

        Ok(messages)
    }
}

impl DataSource for CcSwitchAdapter {
    fn name(&self) -> &str {
        "cc-switch"
    }

    fn enabled(&self) -> bool {
        self.db_path.exists()
    }

    fn collect(&self) -> Result<Vec<UnifiedMessage>, DataSourceError> {
        if !self.enabled() {
            return Err(DataSourceError::NotAvailable(
                "cc-switch.db not found".into(),
            ));
        }
        self.read_messages()
    }
}
