use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use usage_monitor_core::{DailyContribution, ModelStats, SessionSummary, TokenBreakdown, UnifiedMessage};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    client TEXT NOT NULL, model_id TEXT NOT NULL, provider_id TEXT NOT NULL,
    session_id TEXT NOT NULL, timestamp INTEGER NOT NULL,
    input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0, cache_write_tokens INTEGER DEFAULT 0,
    reasoning_tokens INTEGER DEFAULT 0, cost_usd REAL DEFAULT 0,
    request_id TEXT, workspace TEXT, data_source TEXT NOT NULL,
    UNIQUE(request_id, client, timestamp)
);
CREATE TABLE IF NOT EXISTS daily_rollups (
    date TEXT NOT NULL, client TEXT NOT NULL, model_id TEXT NOT NULL,
    provider_id TEXT NOT NULL, request_count INTEGER DEFAULT 0,
    input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0, cache_write_tokens INTEGER DEFAULT 0,
    cost_usd REAL DEFAULT 0,
    PRIMARY KEY (date, client, model_id, provider_id)
);
CREATE TABLE IF NOT EXISTS model_pricing (
    model_id TEXT PRIMARY KEY, display_name TEXT NOT NULL,
    input_cost_per_million REAL NOT NULL, output_cost_per_million REAL NOT NULL,
    cache_read_cost_per_million REAL DEFAULT 0,
    source TEXT NOT NULL, updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(timestamp);
CREATE INDEX IF NOT EXISTS idx_messages_model ON messages(model_id);
CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
";

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap()
    }

    pub fn insert_messages(&self, messages: &[UnifiedMessage]) -> Result<usize, StorageError> {
        let conn = self.lock();
        let tx = conn.unchecked_transaction()?;
        let mut count = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO messages
                 (client,model_id,provider_id,session_id,timestamp,
                  input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,
                  reasoning_tokens,cost_usd,request_id,workspace,data_source)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            )?;
            for msg in messages {
                count += stmt.execute(params![
                    msg.client, msg.model_id, msg.provider_id, msg.session_id,
                    msg.timestamp, msg.tokens.input.max(0), msg.tokens.output.max(0),
                    msg.tokens.cache_read.max(0), msg.tokens.cache_write.max(0),
                    msg.tokens.reasoning.max(0), msg.cost.max(0.0),
                    msg.request_id, msg.workspace, msg.data_source,
                ])?;
            }
        }
        tx.commit()?;
        Ok(count)
    }

    pub fn upsert_daily_rollups(&self) -> Result<(), StorageError> {
        self.lock().execute_batch(
            "INSERT OR REPLACE INTO daily_rollups
             (date,client,model_id,provider_id,request_count,
              input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,cost_usd)
             SELECT date(timestamp/1000,'unixepoch','localtime'), client, model_id,
                    provider_id, COUNT(*), SUM(input_tokens), SUM(output_tokens),
                    SUM(cache_read_tokens), SUM(cache_write_tokens), SUM(cost_usd)
             FROM messages GROUP BY 1,2,3,4",
        )?;
        Ok(())
    }

    pub fn messages_count(&self) -> Result<i64, StorageError> {
        Ok(self.lock().query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?)
    }

    pub fn query_daily(&self) -> Result<Vec<DailyContribution>, StorageError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT date,client,model_id,provider_id,request_count,
                    input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,cost_usd
             FROM daily_rollups ORDER BY date DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok((
            row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?,
            row.get::<_,String>(3)?, row.get::<_,i64>(4)?,
            row.get::<_,i64>(5)?, row.get::<_,i64>(6)?, row.get::<_,i64>(7)?,
            row.get::<_,i64>(8)?, row.get::<_,f64>(9)?,
        )))?;
        Ok(rows.flatten().map(|(date,_,_,_,req_count,in_tok,out_tok,cr_tok,cw_tok,cost)| {
            DailyContribution {
                date, cost,
                request_count: req_count as usize,
                tokens: TokenBreakdown { input:in_tok, output:out_tok, cache_read:cr_tok, cache_write:cw_tok, reasoning:0 },
                by_model: Default::default(),
            }
        }).collect())
    }

    pub fn query_models(&self) -> Result<Vec<ModelStats>, StorageError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT model_id,provider_id,COUNT(DISTINCT session_id),COUNT(*),
                    SUM(input_tokens),SUM(output_tokens),SUM(cache_read_tokens),
                    SUM(cache_write_tokens),SUM(cost_usd)
             FROM messages GROUP BY 1,2 ORDER BY SUM(cost_usd) DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok((
            row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,i64>(2)?,
            row.get::<_,i64>(3)?, row.get::<_,i64>(4)?, row.get::<_,i64>(5)?,
            row.get::<_,i64>(6)?, row.get::<_,i64>(7)?, row.get::<_,f64>(8)?,
        )))?;
        Ok(rows.flatten().map(|(model,prov,sessions,reqs,inp,out,cr,cw,cost)| {
            ModelStats {
                model_id: model, provider_id: prov,
                session_count: sessions as usize, request_count: reqs as usize,
                tokens: TokenBreakdown { input:inp, output:out, cache_read:cr, cache_write:cw, reasoning:0 },
                cost, clients: vec![],
            }
        }).collect())
    }

    pub fn query_sessions(&self) -> Result<Vec<SessionSummary>, StorageError> {
        let conn = self.lock();
        let mut stmt = conn.prepare(
            "SELECT session_id,client,model_id,COUNT(*),
                    SUM(input_tokens),SUM(output_tokens),SUM(cache_read_tokens),
                    SUM(cache_write_tokens),SUM(cost_usd),MIN(timestamp),MAX(timestamp)
             FROM messages GROUP BY 1 ORDER BY MAX(timestamp) DESC LIMIT 100",
        )?;
        let rows = stmt.query_map([], |row| Ok((
            row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?,
            row.get::<_,i64>(3)?, row.get::<_,i64>(4)?, row.get::<_,i64>(5)?,
            row.get::<_,i64>(6)?, row.get::<_,i64>(7)?, row.get::<_,f64>(8)?,
            row.get::<_,i64>(9)?, row.get::<_,i64>(10)?,
        )))?;
        Ok(rows.flatten().map(|(sid,client,model,msg_count,inp,out,cr,cw,cost,first,last)| {
            SessionSummary {
                session_id: sid, client, model_id: model,
                message_count: msg_count as usize, cost,
                tokens: TokenBreakdown { input:inp, output:out, cache_read:cr, cache_write:cw, reasoning:0 },
                first_seen: first, last_seen: last,
            }
        }).collect())
    }
}
