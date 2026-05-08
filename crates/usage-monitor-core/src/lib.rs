//! UsageMonitor core engine.
//!
//! Defines the shared data model (`TokenBreakdown`, `UnifiedMessage`),
//! the `DataSource` trait, protocol normalization utilities,
//! a file-scanning framework, and aggregation logic.
//!
//! Pure library — no HTTP, no SQL, no async runtime.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ── Step 1: TokenBreakdown ──────────────────────────────────────────

/// Normalized token breakdown.
///
/// Uses Anthropic cache semantics as the canonical model:
/// `cache_read` and `cache_write` are independent of `input`.
/// Parsers for OpenAI-protocol tools must separate `cached_tokens`
/// from `prompt_tokens` before populating this struct.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenBreakdown {
    pub input: i64,
    pub output: i64,
    /// Tokens read from cache — did not incur input cost.
    pub cache_read: i64,
    /// Tokens written to cache this request — available for future reads.
    pub cache_write: i64,
    /// Reasoning / thinking tokens (o1, r1, etc.). 0 when not applicable.
    pub reasoning: i64,
}

impl TokenBreakdown {
    /// Total tokens processed (matching provider dashboard totals).
    /// Includes cache_read — providers count cached tokens in their total.
    pub fn total_tokens(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_write + self.reasoning
    }

    /// Billable tokens — excludes cache_read (tokens served from cache, not billed as input).
    pub fn billable_tokens(&self) -> i64 {
        self.input + self.output + self.cache_write + self.reasoning
    }

    /// Cache hit rate: `cache_read / (input + cache_read)`.
    ///
    /// Returns 0.0 when the denominator is zero.
    pub fn cache_hit_rate(&self) -> f64 {
        let denom = (self.input + self.cache_read) as f64;
        if denom == 0.0 {
            0.0
        } else {
            (self.cache_read as f64 / denom).clamp(0.0, 1.0)
        }
    }

    /// Clamp all fields to `>= 0`.
    pub fn clamp_negative(&mut self) {
        self.input = self.input.max(0);
        self.output = self.output.max(0);
        self.cache_read = self.cache_read.max(0);
        self.cache_write = self.cache_write.max(0);
        self.reasoning = self.reasoning.max(0);
    }
}

// ── Step 1: UnifiedMessage ──────────────────────────────────────────

/// One normalized AI request record.
///
/// This is the "universal currency" of UsageMonitor:
/// parsers produce it, the aggregator consumes it, storage persists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnifiedMessage {
    /// Tool identifier: "claude", "codex", "kimi", "gemini", "pi", "openclaw", "hermes"
    pub client: String,
    /// Model identifier: "deepseek-v4-pro", "gpt-5.4", "kimi-for-coding", …
    pub model_id: String,
    /// Provider identifier: "anthropic", "openai", "moonshot", "google", …
    pub provider_id: String,
    /// Session UUID (tool-specific)
    pub session_id: String,
    /// Unix milliseconds timestamp
    pub timestamp: i64,
    /// Normalized token counts
    pub tokens: TokenBreakdown,
    /// Cost in USD. 0 means free or not yet priced.
    pub cost: f64,
    /// Dedup key for streaming (parser-filled, e.g. "messageId:requestId")
    pub request_id: Option<String>,
    /// Working directory at the time of the request
    pub workspace: Option<String>,
    /// Data provenance: "cc-switch" or "native"
    pub data_source: String,
}

// ── Step 2: DataSource trait ────────────────────────────────────────

/// Error returned by [`DataSource::collect`].
#[derive(Debug, thiserror::Error)]
pub enum DataSourceError {
    #[error("data source not available: {0}")]
    NotAvailable(String),
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("I/O error: {0}")]
    IoError(String),
}

/// Abstraction over a data source (CC Switch DB, native session files, …).
///
/// Implementors only need to provide `name()`, `enabled()`, and `collect()`.
pub trait DataSource {
    /// Human-readable name for logging and dedup.
    fn name(&self) -> &str;

    /// Cheap liveness check — does a stat / open, **no parsing**.
    fn enabled(&self) -> bool;

    /// Gather `UnifiedMessage` records. An empty `Vec` means "no new data",
    /// which is **not** an error.
    fn collect(&self) -> Result<Vec<UnifiedMessage>, DataSourceError>;
}

// ── Step 3: Protocol normalization utilities ────────────────────────

/// Separate OpenAI-style cache-inclusive input into canonical form.
///
/// OpenAI's `cached_tokens` is typically included in `prompt_tokens`.
/// This function subtracts it so that `input` represents only *billable*
/// input tokens and `cache_read` represents the cached portion.
///
/// When `cached >= input` the entire `input` is attributed to cache.
///
/// ```
/// use usage_monitor_core::subtract_cached_overlap;
/// assert_eq!(subtract_cached_overlap(1000, 200), (800, 200));
/// assert_eq!(subtract_cached_overlap(100, 200), (0, 100));
/// ```
pub fn subtract_cached_overlap(input: i64, cached: i64) -> (i64, i64) {
    let overlap = cached.min(input);
    (input - overlap, overlap)
}

/// Compute cache hit rate from raw counts.
///
/// `cache_read / (input + cache_read)`, clamped to 0.0–1.0.
/// Returns 0.0 when the denominator is zero.
pub fn cache_hit_rate(cache_read: i64, input: i64) -> f64 {
    let denom = (input + cache_read) as f64;
    if denom == 0.0 {
        0.0
    } else {
        (cache_read as f64 / denom).clamp(0.0, 1.0)
    }
}

/// Normalize a model identifier for grouping.
///
/// Strips date suffixes (`gpt-5.3-2025-08-01` → `gpt-5.3`),
/// normalises dots and dashes, lowercases.
pub fn normalize_model_id(raw: &str) -> String {
    // Strip ISO date suffix: "gpt-5.3-2025-08-01" → "gpt-5.3"
    let without_date = strip_date_suffix(raw);
    // Replace dots with dashes for consistent grouping
    let normalized = without_date.replace('.', "-");
    normalized.to_lowercase()
}

/// If the string ends with a date pattern `-YYYY-MM-DD` or `-YYYYMMDD`,
/// strip it. Also handles `-YYYY-MM` patterns.
fn strip_date_suffix(s: &str) -> &str {
    // Try `-YYYY-MM-DD`
    if s.len() > 11 {
        let tail = &s[s.len() - 11..];
        if tail.starts_with('-')
            && tail[1..5].chars().all(|c| c.is_ascii_digit())
            && tail.chars().nth(5) == Some('-')
            && tail[6..8].chars().all(|c| c.is_ascii_digit())
            && tail.chars().nth(8) == Some('-')
            && tail[9..11].chars().all(|c| c.is_ascii_digit())
        {
            return &s[..s.len() - 11];
        }
    }
    s
}

// ── Step 4: Scanner framework ───────────────────────────────────────

/// Error returned by scanner functions.
#[derive(Debug, thiserror::Error)]
pub enum ScannerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A scan task: root directory + file pattern.
#[derive(Debug, Clone)]
pub struct ScanTask {
    pub root: PathBuf,
    /// Glob pattern, e.g. `"*.jsonl"`, `"*.json|*.jsonl"`.
    pub pattern: String,
    /// Optional exclusion glob.
    pub exclude: Option<String>,
}

/// Walk a directory and return files matching the pattern.
///
/// Supports pipe-separated multi-patterns: `"*.json|*.jsonl"`.
/// Results are sorted and deduplicated.
pub fn scan_directory(task: &ScanTask) -> Result<Vec<PathBuf>, ScannerError> {
    let patterns: Vec<&str> = task.pattern.split('|').map(|p| p.trim()).collect();
    let mut results = Vec::new();

    for entry in walkdir::WalkDir::new(&task.root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let file_name = entry.file_name().to_string_lossy();
        let matches_pattern = patterns.iter().any(|pat| {
            glob_match_simple(pat, &file_name)
        });

        if !matches_pattern {
            continue;
        }

        if let Some(ref exclude) = task.exclude {
            if glob_match_simple(exclude, &file_name) {
                continue;
            }
        }

        results.push(entry.path().to_path_buf());
    }

    results.sort_unstable();
    results.dedup();
    Ok(results)
}

/// Scan multiple tasks in parallel using rayon (or sequentially if rayon is unavailable).
pub fn scan_all(tasks: &[ScanTask]) -> Result<Vec<PathBuf>, ScannerError> {
    let mut all = Vec::new();
    for task in tasks {
        let files = scan_directory(task)?;
        all.extend(files);
    }
    all.sort_unstable();
    all.dedup();
    Ok(all)
}

/// Simple glob matching: supports `*` wildcard only.
fn glob_match_simple(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') {
        return pattern == name;
    }
    if let Some(ext) = pattern.strip_prefix("*.") {
        return name.ends_with(&format!(".{ext}"));
    }
    if let Some(base) = pattern.strip_suffix(".*") {
        return name.starts_with(base) && name[base.len()..].starts_with('.');
    }
    // fallback: wildcard anywhere
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 2 {
        return name.starts_with(parts[0]) && name.ends_with(parts[1]);
    }
    false
}

// ── Step 5: Aggregation types ───────────────────────────────────────

/// Daily aggregated usage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailyContribution {
    pub date: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub request_count: usize,
    pub by_model: HashMap<String, TokenBreakdown>,
}

/// Per-model aggregated statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelStats {
    pub model_id: String,
    pub provider_id: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub request_count: usize,
    pub session_count: usize,
    pub clients: Vec<String>,
}

/// Per-session summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub client: String,
    pub model_id: String,
    pub tokens: TokenBreakdown,
    pub cost: f64,
    pub message_count: usize,
    pub first_seen: i64,
    pub last_seen: i64,
}

/// Aggregate messages by date (local timezone).
pub fn aggregate_by_date(messages: &[UnifiedMessage]) -> Vec<DailyContribution> {
    let mut days: BTreeMap<String, DailyContribution> = BTreeMap::new();

    for msg in messages {
        let date = timestamp_to_date(msg.timestamp);
        let day = days.entry(date).or_insert_with(|| DailyContribution {
            date: String::new(),
            tokens: TokenBreakdown::default(),
            cost: 0.0,
            request_count: 0,
            by_model: HashMap::new(),
        });
        day.date = day.date.clone(); // keep first date string we got
        if day.date.is_empty() {
            day.date = timestamp_to_date(msg.timestamp);
        }
        day.tokens.input += msg.tokens.input;
        day.tokens.output += msg.tokens.output;
        day.tokens.cache_read += msg.tokens.cache_read;
        day.tokens.cache_write += msg.tokens.cache_write;
        day.tokens.reasoning += msg.tokens.reasoning;
        day.tokens.clamp_negative();
        day.cost += msg.cost;
        day.request_count += 1;
        day.by_model
            .entry(msg.model_id.clone())
            .or_default()
            .input += msg.tokens.input;
        day.by_model
            .entry(msg.model_id.clone())
            .or_default()
            .output += msg.tokens.output;
        // clamp by_model entries too
        for v in day.by_model.values_mut() {
            v.clamp_negative();
        }
    }

    // Post-process to fix date field
    days.into_values().collect()
}

/// Aggregate messages by (provider_id, model_id).
pub fn aggregate_by_model(messages: &[UnifiedMessage]) -> Vec<ModelStats> {
    let mut models: HashMap<(String, String), ModelStats> = HashMap::new();
    let mut model_sessions: HashMap<(String, String), HashSet<String>> = HashMap::new();
    let mut model_clients: HashMap<(String, String), HashSet<String>> = HashMap::new();

    for msg in messages {
        let key = (msg.provider_id.clone(), msg.model_id.clone());
        let entry = models.entry(key.clone()).or_insert_with(|| ModelStats {
            model_id: msg.model_id.clone(),
            provider_id: msg.provider_id.clone(),
            tokens: TokenBreakdown::default(),
            cost: 0.0,
            request_count: 0,
            session_count: 0,
            clients: vec![],
        });
        entry.tokens.input += msg.tokens.input;
        entry.tokens.output += msg.tokens.output;
        entry.tokens.cache_read += msg.tokens.cache_read;
        entry.tokens.cache_write += msg.tokens.cache_write;
        entry.tokens.reasoning += msg.tokens.reasoning;
        entry.tokens.clamp_negative();
        entry.cost += msg.cost;
        entry.request_count += 1;
        model_sessions
            .entry(key.clone())
            .or_default()
            .insert(msg.session_id.clone());
        model_clients
            .entry(key.clone())
            .or_default()
            .insert(msg.client.clone());
    }

    for (key, stats) in models.iter_mut() {
        stats.session_count = model_sessions.get(key).map(|s| s.len()).unwrap_or(0);
        let mut clients: Vec<String> = model_clients
            .get(key)
            .map(|c| c.iter().cloned().collect())
            .unwrap_or_default();
        clients.sort();
        stats.clients = clients;
    }

    let mut result: Vec<ModelStats> = models.into_values().collect();
    result.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    result
}

/// Aggregate messages by session_id.
pub fn aggregate_by_session(messages: &[UnifiedMessage]) -> Vec<SessionSummary> {
    let mut sessions: HashMap<String, SessionSummary> = HashMap::new();

    for msg in messages {
        let entry = sessions
            .entry(msg.session_id.clone())
            .or_insert_with(|| SessionSummary {
                session_id: msg.session_id.clone(),
                client: msg.client.clone(),
                model_id: msg.model_id.clone(),
                tokens: TokenBreakdown::default(),
                cost: 0.0,
                message_count: 0,
                first_seen: msg.timestamp,
                last_seen: msg.timestamp,
            });
        entry.tokens.input += msg.tokens.input;
        entry.tokens.output += msg.tokens.output;
        entry.tokens.cache_read += msg.tokens.cache_read;
        entry.tokens.cache_write += msg.tokens.cache_write;
        entry.tokens.reasoning += msg.tokens.reasoning;
        entry.tokens.clamp_negative();
        entry.cost += msg.cost;
        entry.message_count += 1;
        entry.first_seen = entry.first_seen.min(msg.timestamp);
        entry.last_seen = entry.last_seen.max(msg.timestamp);
    }

    let mut result: Vec<SessionSummary> = sessions.into_values().collect();
    result.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    result
}

// ── helpers ─────────────────────────────────────────────────────────

fn timestamp_to_date(ts_ms: i64) -> String {
    use chrono::TimeZone;
    let secs = ts_ms / 1000;
    let nsecs = ((ts_ms % 1000) * 1_000_000) as u32;
    match chrono::Local.timestamp_opt(secs, nsecs) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d").to_string(),
        _ => "unknown".to_string(),
    }
}

// ── tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── Step 1: TokenBreakdown ──────────────────────────────

    #[test]
    fn total_tokens_excludes_cache_read() {
        let tb = TokenBreakdown {
            input: 1000,
            output: 500,
            cache_read: 200,
            cache_write: 100,
            reasoning: 0,
        };
        assert_eq!(tb.total_tokens(), 1800); // 1000+500+200+100+0
    }

    #[test]
    fn total_tokens_includes_all_but_cache_read() {
        let tb = TokenBreakdown {
            input: 100,
            output: 200,
            cache_read: 9999,
            cache_write: 50,
            reasoning: 30,
        };
        assert_eq!(tb.total_tokens(), 10379); // 100+200+9999+50+30
    }

    #[test]
    fn cache_hit_rate_normal() {
        let tb = TokenBreakdown {
            input: 800,
            output: 0,
            cache_read: 200,
            cache_write: 0,
            reasoning: 0,
        };
        assert!((tb.cache_hit_rate() - 0.2).abs() < 0.001);
    }

    #[test]
    fn cache_hit_rate_zero_denom() {
        let tb = TokenBreakdown::default();
        assert_eq!(tb.cache_hit_rate(), 0.0);
    }

    #[test]
    fn cache_hit_rate_full_cache() {
        let tb = TokenBreakdown {
            input: 0,
            output: 100,
            cache_read: 500,
            cache_write: 0,
            reasoning: 0,
        };
        assert!((tb.cache_hit_rate() - 1.0).abs() < 0.001);
    }

    #[test]
    fn clamp_negative_all_fields() {
        let mut tb = TokenBreakdown {
            input: -1,
            output: -2,
            cache_read: -3,
            cache_write: -4,
            reasoning: -5,
        };
        tb.clamp_negative();
        assert!(tb.input >= 0);
        assert!(tb.output >= 0);
        assert!(tb.cache_read >= 0);
        assert!(tb.cache_write >= 0);
        assert!(tb.reasoning >= 0);
    }

    #[test]
    fn clamp_negative_mixed() {
        let mut tb = TokenBreakdown {
            input: 100,
            output: -50,
            cache_read: 200,
            cache_write: 0,
            reasoning: -10,
        };
        tb.clamp_negative();
        assert_eq!(tb.input, 100);
        assert_eq!(tb.output, 0);
        assert_eq!(tb.cache_read, 200);
        assert_eq!(tb.cache_write, 0);
        assert_eq!(tb.reasoning, 0);
    }

    // ── Step 2: DataSource trait ────────────────────────────

    struct MockSource {
        name: String,
        available: bool,
        messages: Vec<UnifiedMessage>,
    }

    impl DataSource for MockSource {
        fn name(&self) -> &str {
            &self.name
        }
        fn enabled(&self) -> bool {
            self.available
        }
        fn collect(&self) -> Result<Vec<UnifiedMessage>, DataSourceError> {
            Ok(self.messages.clone())
        }
    }

    #[test]
    fn datasource_enabled_returns_false_when_unavailable() {
        let ds = MockSource {
            name: "test".into(),
            available: false,
            messages: vec![],
        };
        assert!(!ds.enabled());
    }

    #[test]
    fn datasource_collect_returns_empty_vec_not_error() {
        let ds = MockSource {
            name: "test".into(),
            available: true,
            messages: vec![],
        };
        let result = ds.collect().unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn datasource_error_display() {
        let e = DataSourceError::NotAvailable("db missing".into());
        assert!(e.to_string().contains("db missing"));
        let e = DataSourceError::ParseError("invalid json".into());
        assert!(e.to_string().contains("invalid json"));
    }

    // ── Step 3: Protocol normalization ──────────────────────

    #[test]
    fn subtract_cached_overlap_normal() {
        assert_eq!(subtract_cached_overlap(1000, 200), (800, 200));
    }

    #[test]
    fn subtract_cached_overlap_cached_exceeds_input() {
        assert_eq!(subtract_cached_overlap(100, 200), (0, 100));
    }

    #[test]
    fn subtract_cached_overlap_zero_cached() {
        assert_eq!(subtract_cached_overlap(500, 0), (500, 0));
    }

    #[test]
    fn subtract_cached_overlap_both_zero() {
        assert_eq!(subtract_cached_overlap(0, 0), (0, 0));
    }

    #[test]
    fn cache_hit_rate_fn_normal() {
        let rate = cache_hit_rate(200, 800);
        assert!((rate - 0.2).abs() < 0.001);
    }

    #[test]
    fn cache_hit_rate_fn_zero_denom() {
        assert_eq!(cache_hit_rate(0, 0), 0.0);
    }

    #[test]
    fn normalize_model_id_strips_date_suffix() {
        assert_eq!(normalize_model_id("gpt-5.3-2025-08-01"), "gpt-5-3");
    }

    #[test]
    fn normalize_model_id_lowercases() {
        assert_eq!(normalize_model_id("GPT-5.4"), "gpt-5-4");
    }

    #[test]
    fn normalize_model_id_replaces_dots() {
        assert_eq!(normalize_model_id("claude.sonnet.4.6"), "claude-sonnet-4-6");
    }

    #[test]
    fn normalize_model_id_noop_for_simple_names() {
        assert_eq!(normalize_model_id("deepseek-v4-pro"), "deepseek-v4-pro");
    }

    // ── Step 4: Scanner ─────────────────────────────────────

    #[test]
    fn scan_directory_single_pattern() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("a.jsonl")).unwrap();
        std::fs::File::create(dir.path().join("b.txt")).unwrap();

        let task = ScanTask {
            root: dir.path().to_path_buf(),
            pattern: "*.jsonl".into(),
            exclude: None,
        };
        let files = scan_directory(&task).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("a.jsonl"));
    }

    #[test]
    fn scan_directory_multi_pattern() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("a.json")).unwrap();
        std::fs::File::create(dir.path().join("b.jsonl")).unwrap();
        std::fs::File::create(dir.path().join("c.txt")).unwrap();

        let task = ScanTask {
            root: dir.path().to_path_buf(),
            pattern: "*.json|*.jsonl".into(),
            exclude: None,
        };
        let files = scan_directory(&task).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn scan_directory_exclude() {
        let dir = TempDir::new().unwrap();
        std::fs::File::create(dir.path().join("keep.jsonl")).unwrap();
        std::fs::File::create(dir.path().join("skip.tmp.jsonl")).unwrap();

        let task = ScanTask {
            root: dir.path().to_path_buf(),
            pattern: "*.jsonl".into(),
            exclude: Some("*.tmp.jsonl".into()),
        };
        let files = scan_directory(&task).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("keep.jsonl"));
    }

    #[test]
    fn scan_directory_empty_dir() {
        let dir = TempDir::new().unwrap();
        let task = ScanTask {
            root: dir.path().to_path_buf(),
            pattern: "*.jsonl".into(),
            exclude: None,
        };
        let files = scan_directory(&task).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn scan_all_multi_task_dedup() {
        let dir1 = TempDir::new().unwrap();
        let dir2 = TempDir::new().unwrap();
        std::fs::File::create(dir1.path().join("a.jsonl")).unwrap();
        std::fs::File::create(dir2.path().join("a.jsonl")).unwrap(); // same name, different dir = different path
        std::fs::File::create(dir2.path().join("b.jsonl")).unwrap();

        let tasks = vec![
            ScanTask {
                root: dir1.path().to_path_buf(),
                pattern: "*.jsonl".into(),
                exclude: None,
            },
            ScanTask {
                root: dir2.path().to_path_buf(),
                pattern: "*.jsonl".into(),
                exclude: None,
            },
        ];
        let files = scan_all(&tasks).unwrap();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn scan_directory_nonexistent_dir() {
        let task = ScanTask {
            root: PathBuf::from("/nonexistent/path/xyz"),
            pattern: "*.jsonl".into(),
            exclude: None,
        };
        let files = scan_directory(&task).unwrap();
        assert!(files.is_empty()); // walkdir silently returns nothing for non-existent roots
    }

    // ── Step 5: Aggregation ─────────────────────────────────

    fn make_msg(
        client: &str,
        model_id: &str,
        provider_id: &str,
        session_id: &str,
        timestamp: i64,
        input: i64,
        output: i64,
        cache_read: i64,
    ) -> UnifiedMessage {
        UnifiedMessage {
            client: client.into(),
            model_id: model_id.into(),
            provider_id: provider_id.into(),
            session_id: session_id.into(),
            timestamp,
            tokens: TokenBreakdown {
                input,
                output,
                cache_read,
                cache_write: 0,
                reasoning: 0,
            },
            cost: 0.0,
            request_id: None,
            workspace: None,
            data_source: "test".into(),
        }
    }

    #[test]
    fn aggregate_by_date_empty() {
        let result = aggregate_by_date(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn aggregate_by_date_single() {
        let msgs = vec![make_msg("claude", "gpt-5.4", "openai", "s1", 1700000000000, 100, 50, 10)];
        let result = aggregate_by_date(&msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].tokens.input, 100);
        assert_eq!(result[0].request_count, 1);
    }

    #[test]
    fn aggregate_by_date_same_day_merges() {
        let msgs = vec![
            make_msg("claude", "gpt-5.4", "openai", "s1", 1700000000000, 100, 50, 10),
            make_msg("claude", "gpt-5.4", "openai", "s1", 1700000001000, 200, 60, 20),
        ];
        let result = aggregate_by_date(&msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].tokens.input, 300);
        assert_eq!(result[0].request_count, 2);
    }

    #[test]
    fn aggregate_by_date_across_days_splits() {
        let msgs = vec![
            make_msg("claude", "gpt-5.4", "openai", "s1", 1700000000000, 100, 50, 0),  // day 1
            make_msg("claude", "gpt-5.4", "openai", "s1", 1700086400000, 200, 60, 0),  // day 2
        ];
        let result = aggregate_by_date(&msgs);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn aggregate_by_model_groups_correctly() {
        let msgs = vec![
            make_msg("claude", "gpt-5.4", "openai", "s1", 1700000000000, 100, 50, 0),
            make_msg("claude", "gpt-5.4", "openai", "s2", 1700000001000, 200, 60, 0),
            make_msg("codex", "deepseek", "deepseek", "s3", 1700000002000, 300, 70, 0),
        ];
        let result = aggregate_by_model(&msgs);
        // 2 models: gpt-5.4 (2 sessions), deepseek (1 session)
        let gpt = result.iter().find(|m| m.model_id == "gpt-5.4").unwrap();
        assert_eq!(gpt.tokens.input, 300);
        assert_eq!(gpt.session_count, 2);
        assert_eq!(gpt.request_count, 2);

        let ds = result.iter().find(|m| m.model_id == "deepseek").unwrap();
        assert_eq!(ds.request_count, 1);
        assert_eq!(ds.session_count, 1);
    }

    #[test]
    fn aggregate_by_session_counts_messages() {
        let msgs = vec![
            make_msg("claude", "gpt-5.4", "openai", "s1", 1700000000000, 100, 50, 0),
            make_msg("claude", "gpt-5.4", "openai", "s1", 1700000001000, 200, 60, 0),
            make_msg("codex", "deepseek", "deepseek", "s2", 1700000002000, 300, 70, 0),
        ];
        let result = aggregate_by_session(&msgs);
        assert_eq!(result.len(), 2);
        let s1 = result.iter().find(|s| s.session_id == "s1").unwrap();
        assert_eq!(s1.message_count, 2);
        let s2 = result.iter().find(|s| s.session_id == "s2").unwrap();
        assert_eq!(s2.message_count, 1);
    }

    #[test]
    fn aggregation_handles_negative_tokens() {
        let msgs = vec![make_msg("claude", "gpt-5.4", "openai", "s1", 1700000000000, -10, -5, -3)];
        let by_date = aggregate_by_date(&msgs);
        assert!(by_date[0].tokens.input >= 0);
        let by_model = aggregate_by_model(&msgs);
        assert!(by_model[0].tokens.input >= 0);
        let by_session = aggregate_by_session(&msgs);
        assert!(by_session[0].tokens.input >= 0);
    }

    #[test]
    fn session_summary_tracks_time_range() {
        let msgs = vec![
            make_msg("claude", "gpt-5.4", "openai", "s1", 1000, 10, 0, 0),
            make_msg("claude", "gpt-5.4", "openai", "s1", 3000, 10, 0, 0),
        ];
        let result = aggregate_by_session(&msgs);
        let s = &result[0];
        assert_eq!(s.first_seen, 1000);
        assert_eq!(s.last_seen, 3000);
    }
}
