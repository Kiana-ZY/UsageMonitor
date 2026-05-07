use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde::Deserialize;
use usage_monitor_cc_switch::CcSwitchAdapter;
use usage_monitor_core::DataSource;
use usage_monitor_parsers as parsers;
use usage_monitor_pricing::PricingEngine;
use usage_monitor_storage::Storage;
use usage_monitor_tui::run as tui_run;
use usage_monitor_web::serve as web_serve;

/// UsageMonitor — AI coding tool token usage tracker.
///
/// Unified dashboard for token consumption and cost across
/// Claude Code, Codex, Kimi Code, Gemini CLI, Pi, OpenClaw, Hermess, and more.
#[derive(Parser)]
#[command(name = "usage-monitor", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the web dashboard
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "4317")]
        port: u16,
    },
    /// Start the terminal UI
    Tui,
    /// Trigger a manual scan of all data sources
    Scan,
}

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub port: Option<u16>,
    pub data_sources: Option<Vec<String>>,
    pub cc_switch_db: Option<String>,
}

fn config_path() -> PathBuf {
    dirs_next().join(".usage-monitor").join("config.toml")
}

fn dirs_next() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_default()
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
    }
}

fn load_config() -> Config {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => match toml::from_str(&content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!(
                    "warning: failed to parse {}: {}. Using defaults.",
                    path.display(),
                    e
                );
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve { port } => {
            let _config = load_config();
            let home = dirs_next();
            let db_path = home.join(".usage-monitor").join("usage-monitor.db");
            std::fs::create_dir_all(db_path.parent().unwrap())?;
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(web_serve(db_path, home, port))?;
        }
        Command::Tui => {
            let _config = load_config();
            let home = dirs_next();
            let db_path = home.join(".usage-monitor").join("usage-monitor.db");
            tui_run(db_path, home)?;
        }
        Command::Scan => {
            let _config = load_config();
            println!("Scanning data sources...");

            let home = dirs_next();
            let claude_dir = home.join(".claude").join("projects");
            let mut all_messages = Vec::new();

            // CC Switch data source
            let cc_switch = CcSwitchAdapter::new(CcSwitchAdapter::default_path());
            if cc_switch.enabled() {
                println!("CC Switch DB found — collecting proxy data...");
                match cc_switch.collect() {
                    Ok(cc_msgs) => {
                        println!("  CC Switch: {} messages", cc_msgs.len());
                        all_messages.extend(cc_msgs);
                    }
                    Err(e) => eprintln!("  CC Switch error: {}", e),
                }
            }

            // Native parsers
            let native = parsers::parse_all(
                if claude_dir.exists() { Some(claude_dir.as_path()) } else { None },
                None, // codex
                None, // kimi
                None, // gemini
                None, // pi
                None, // openclaw
                None, // hermes
            );
            all_messages.extend(native);

            println!("Parsed {} messages across {} tools.",
                all_messages.len(),
                all_messages.iter().map(|m| m.client.as_str()).collect::<std::collections::HashSet<_>>().len()
            );

            // Show quick summary
            for msg in &all_messages {
                println!(
                    "  [{}] model={} input={} output={} cache_read={} cache_write={}",
                    msg.client,
                    msg.model_id,
                    msg.tokens.input,
                    msg.tokens.output,
                    msg.tokens.cache_read,
                    msg.tokens.cache_write,
                );
            }

            // Save to storage
            if !all_messages.is_empty() {
                let db_path = home.join(".usage-monitor").join("usage-monitor.db");
                std::fs::create_dir_all(db_path.parent().unwrap()).ok();
                match Storage::open(&db_path) {
                    Ok(storage) => {
                        let inserted = storage.insert_messages(&all_messages).unwrap_or(0);
                        storage.upsert_daily_rollups().ok();
                        println!("\nSaved {} new messages to {}", inserted, db_path.display());
                    }
                    Err(e) => eprintln!("\nStorage error: {}", e),
                }
            }

            if !all_messages.is_empty() {
                let total_input: i64 = all_messages.iter().map(|m| m.tokens.input).sum();
                let total_output: i64 = all_messages.iter().map(|m| m.tokens.output).sum();
                let total_cache_read: i64 = all_messages.iter().map(|m| m.tokens.cache_read).sum();
                println!();
                println!("Totals:");
                println!("  input:      {}", total_input);
                println!("  output:     {}", total_output);
                println!("  cache_read: {}", total_cache_read);
                println!("  cache_hit_rate: {:.1}%",
                    usage_monitor_core::cache_hit_rate(total_cache_read, total_input) * 100.0);

                // Calculate cost with pricing engine
                let cc_db = home.join(".cc-switch").join("cc-switch.db");
                let engine = PricingEngine::new(if cc_db.exists() { cc_db.to_str() } else { None });
                let mut total_cost = 0.0;
                for msg in &all_messages {
                    total_cost += engine.calculate_cost(&msg.model_id, &msg.tokens);
                }
                println!("  estimated_cost: ${:.2}", total_cost);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_all_none() {
        let c = Config::default();
        assert!(c.port.is_none());
        assert!(c.data_sources.is_none());
        assert!(c.cc_switch_db.is_none());
    }

    #[test]
    fn config_deserialize_from_toml() {
        let toml_str = r#"
port = 3000
data_sources = ["cc-switch", "native"]
cc_switch_db = "/custom/path/cc-switch.db"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.port, Some(3000));
        assert_eq!(
            config.data_sources,
            Some(vec!["cc-switch".to_string(), "native".to_string()])
        );
        assert_eq!(
            config.cc_switch_db,
            Some("/custom/path/cc-switch.db".to_string())
        );
    }

    #[test]
    fn config_partial_toml() {
        let toml_str = r#"port = 8080"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.port, Some(8080));
        assert!(config.data_sources.is_none());
    }

    #[test]
    fn config_from_empty_is_default() {
        let config: Config = toml::from_str("").unwrap_or_default();
        assert!(config.port.is_none());
    }
}
