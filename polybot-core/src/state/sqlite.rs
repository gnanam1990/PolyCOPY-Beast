use polybot_common::errors::PolybotError;
use polybot_common::types::{Category, Position, PositionStatus, Side, Trade};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr as _;

/// Row to be written to the `signal_log` table. Mirrors the INSERT
/// column order used by [`SqliteStore::insert_signal_log`]. Borrows
/// all string fields so callers don't need to allocate.
pub struct SignalLogInsert<'a> {
    pub signal_id: &'a str,
    pub timestamp: &'a str,
    pub wallet_address: &'a str,
    pub market_id: &'a str,
    pub confidence: u8,
    pub secret_level: u8,
    pub category: &'a str,
    pub side: &'a str,
    pub disposition: &'a str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalLogEntry {
    pub signal_id: String,
    pub timestamp: String,
    pub wallet_address: String,
    pub market_id: String,
    pub confidence: u8,
    pub secret_level: u8,
    pub category: String,
    pub side: String,
    pub disposition: String,
    pub received_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSignalRow {
    pub id: String,
    pub source: String,
    pub tx_hash: Option<String>,
    pub received_at: String,
    pub target_wallet: String,
    pub market_id: String,
    pub token_id: String,
    pub side: String,
    pub direction: String,
    pub outcome: String,
    pub target_price: Decimal,
    pub target_size: Decimal,
    pub confidence: u8,
    pub secret_level: u8,
    pub category: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedPositionRow {
    pub position: Position,
    pub current_price: Option<Decimal>,
    pub unrealized_pnl: Option<Decimal>,
    pub owned_by_wallet: Option<String>,
    pub last_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketMetadataRow {
    pub condition_id: String,
    pub question: Option<String>,
    pub slug: Option<String>,
    pub icon: Option<String>,
    pub resolved: bool,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DailyStatsRow {
    pub date: String,
    pub starting_balance: Decimal,
    pub realized_pnl: Decimal,
    pub unrealized_pnl: Decimal,
    pub volume_traded: Decimal,
    pub trades_placed: u32,
    pub trades_filled: u32,
    pub trades_rejected: u32,
    pub drawdown_pct: Decimal,
    pub paused_at: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TargetRow {
    pub wallet_address: String,
    pub label: Option<String>,
    pub categories: Vec<Category>,
    pub score: Option<Decimal>,
    pub active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentTradeRow {
    pub id: String,
    pub signal_id: String,
    pub source_wallet: String,
    pub market_id: String,
    pub category: String,
    pub side: String,
    pub direction: String,
    pub status: String,
    pub size_usd: Decimal,
    pub placed_at: String,
    pub simulated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CopiedLotRow {
    pub id: String,
    pub source_wallet: String,
    pub market_id: String,
    pub side: String,
    pub current_size: Decimal,
    pub average_price: Decimal,
    pub opened_at: String,
    pub updated_at: String,
    pub last_signal_id: Option<String>,
    pub last_tx_hash: Option<String>,
}

/// v2.5: SQLite cold persistence for trade history and audit trail.
pub struct SqliteStore {
    conn: rusqlite::Connection,
}

impl SqliteStore {
    fn canonicalize_copied_lot_side(side: &str) -> String {
        match side.to_ascii_uppercase().as_str() {
            "YES" => "YES".to_string(),
            "NO" => "NO".to_string(),
            other => other.to_string(),
        }
    }

    pub fn open(db_path: &Path) -> Result<Self, PolybotError> {
        let conn = rusqlite::Connection::open(db_path)
            .map_err(|e| PolybotError::State(format!("Failed to open SQLite: {}", e)))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| PolybotError::State(format!("Failed to enable FK enforcement: {}", e)))?;
        let store = Self { conn };
        store.create_tables()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self, PolybotError> {
        let conn = rusqlite::Connection::open_in_memory()
            .map_err(|e| PolybotError::State(format!("Failed to open in-memory SQLite: {}", e)))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| PolybotError::State(format!("Failed to enable FK enforcement: {}", e)))?;
        let store = Self { conn };
        store.create_tables()?;
        Ok(store)
    }

    fn create_tables(&self) -> Result<(), PolybotError> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS trades (
                id TEXT PRIMARY KEY,
                signal_id TEXT NOT NULL,
                source_wallet TEXT NOT NULL DEFAULT '',
                market_id TEXT NOT NULL,
                category TEXT,
                side TEXT NOT NULL,
                direction TEXT NOT NULL DEFAULT 'Buy',
                price TEXT NOT NULL,
                size TEXT NOT NULL,
                size_usd TEXT NOT NULL,
                filled_size TEXT NOT NULL,
                order_type TEXT NOT NULL,
                status TEXT NOT NULL,
                placed_at TEXT NOT NULL,
                filled_at TEXT,
                simulated INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS positions (
                id TEXT PRIMARY KEY,
                market_id TEXT NOT NULL,
                side TEXT NOT NULL,
                entry_price TEXT NOT NULL,
                current_size TEXT NOT NULL,
                average_price TEXT NOT NULL,
                opened_at TEXT NOT NULL,
                status TEXT NOT NULL,
                category TEXT NOT NULL,
                current_price TEXT,
                unrealized_pnl TEXT,
                owned_by_wallet TEXT,
                last_updated TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS signal_log (
                signal_id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                wallet_address TEXT NOT NULL,
                market_id TEXT NOT NULL,
                confidence INTEGER NOT NULL,
                secret_level INTEGER NOT NULL,
                category TEXT NOT NULL,
                side TEXT NOT NULL,
                disposition TEXT NOT NULL,
                received_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS signals (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                tx_hash TEXT UNIQUE,
                received_at TEXT NOT NULL,
                target_wallet TEXT NOT NULL,
                market_id TEXT NOT NULL,
                token_id TEXT NOT NULL,
                side TEXT NOT NULL,
                direction TEXT NOT NULL DEFAULT 'Buy',
                outcome TEXT NOT NULL,
                target_price TEXT NOT NULL,
                target_size TEXT NOT NULL,
                confidence INTEGER NOT NULL,
                secret_level INTEGER NOT NULL,
                category TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending'
            );
            CREATE TABLE IF NOT EXISTS targets (
                wallet_address TEXT PRIMARY KEY,
                label TEXT,
                added_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                active INTEGER NOT NULL DEFAULT 1,
                categories TEXT NOT NULL DEFAULT '[]',
                score TEXT,
                notes TEXT
            );
            CREATE TABLE IF NOT EXISTS daily_stats (
                date TEXT PRIMARY KEY,
                starting_balance TEXT NOT NULL,
                realized_pnl TEXT NOT NULL DEFAULT '0',
                unrealized_pnl TEXT NOT NULL DEFAULT '0',
                volume_traded TEXT NOT NULL DEFAULT '0',
                trades_placed INTEGER NOT NULL DEFAULT 0,
                trades_filled INTEGER NOT NULL DEFAULT 0,
                trades_rejected INTEGER NOT NULL DEFAULT 0,
                drawdown_pct TEXT NOT NULL DEFAULT '0',
                paused_at TEXT,
                notes TEXT
            );
            CREATE TABLE IF NOT EXISTS config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS market_metadata (
                 condition_id TEXT PRIMARY KEY,
                 question TEXT,
                 slug TEXT,
                 icon TEXT,
                 resolved INTEGER NOT NULL DEFAULT 0,
                 fetched_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS copied_lots (
                 id TEXT PRIMARY KEY,
                 source_wallet TEXT NOT NULL,
                 market_id TEXT NOT NULL,
                 side TEXT NOT NULL,
                 current_size TEXT NOT NULL,
                 average_price TEXT NOT NULL,
                 opened_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL,
                 last_signal_id TEXT,
                 last_tx_hash TEXT,
                 UNIQUE(source_wallet, market_id, side)
             );",
            )
            .map_err(|e| PolybotError::State(format!("Failed to create tables: {}", e)))?;
        self.ensure_column("signals", "direction", "TEXT NOT NULL DEFAULT 'Buy'")?;
        self.ensure_column("trades", "source_wallet", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("trades", "direction", "TEXT NOT NULL DEFAULT 'Buy'")?;
        self.run_migrations()?;
        Ok(())
    }

    fn ensure_column(
        &self,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<(), PolybotError> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({})", table))
            .map_err(|e| {
                PolybotError::State(format!("Failed to inspect {} columns: {}", table, e))
            })?;
        let exists = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| PolybotError::State(format!("Failed to query {} columns: {}", table, e)))?
            .filter_map(Result::ok)
            .any(|name| name == column);

        if !exists {
            self.conn
                .execute(
                    &format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, definition),
                    [],
                )
                .map_err(|e| {
                    PolybotError::State(format!("Failed to add {}.{} column: {}", table, column, e))
                })?;
        }

        Ok(())
    }

    fn run_migrations(&self) -> Result<(), PolybotError> {
        use crate::state::migrations::MIGRATIONS;

        let current: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|e| PolybotError::State(format!("Failed to read schema_migrations: {}", e)))?;

        for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
            tracing::info!(
                version = migration.version,
                description = migration.description,
                "Applying schema migration"
            );
            let tx = self.conn.unchecked_transaction().map_err(|e| {
                PolybotError::State(format!(
                    "Failed to begin migration v{} ({}) transaction: {}",
                    migration.version, migration.description, e
                ))
            })?;
            tx.execute_batch(migration.sql).map_err(|e| {
                PolybotError::State(format!(
                    "Migration v{} ({}) failed: {}",
                    migration.version, migration.description, e
                ))
            })?;
            tx.execute(
                "INSERT INTO schema_migrations (version) VALUES (?1)",
                [migration.version],
            )
            .map_err(|e| {
                PolybotError::State(format!(
                    "Failed to record schema_migrations v{}: {}",
                    migration.version, e
                ))
            })?;
            tx.commit().map_err(|e| {
                PolybotError::State(format!(
                    "Failed to commit migration v{} ({}): {}",
                    migration.version, migration.description, e
                ))
            })?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn run_migrations_for_test(&self) -> Result<(), PolybotError> {
        self.run_migrations()
    }

    pub fn insert_trade(&self, trade: &Trade) -> Result<(), PolybotError> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO trades (
                id, signal_id, source_wallet, market_id, category, side, direction, price, size,
                size_usd, filled_size, order_type, status, placed_at, filled_at, simulated,
                transaction_id, transaction_hash, relayer_state,
                taker_fee_bps, fee_paid_usdc, rebate_usdc, retry_count, error_msg
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19,
                ?20, ?21, ?22, ?23, ?24
            )",
                rusqlite::params![
                    trade.id,
                    trade.signal_id,
                    trade.source_wallet,
                    trade.market_id,
                    trade.category.to_string(),
                    format!("{:?}", trade.side),
                    format!("{:?}", trade.direction),
                    trade.price.to_string(),
                    trade.size.to_string(),
                    trade.size_usd.to_string(),
                    trade.filled_size.to_string(),
                    format!("{:?}", trade.order_type),
                    format!("{:?}", trade.status),
                    trade.placed_at.to_rfc3339(),
                    trade.filled_at.map(|t| t.to_rfc3339()),
                    trade.simulated as i32,
                    trade.transaction_id,
                    trade.transaction_hash,
                    trade.relayer_state.map(|s| s.as_sqlite_str().to_string()),
                    trade.taker_fee_bps as i64,
                    trade.fee_paid_usdc.to_string(),
                    trade.rebate_usdc.to_string(),
                    trade.retry_count as i64,
                    trade.error_msg,
                ],
            )
            .map_err(|e| PolybotError::State(format!("Failed to insert trade: {}", e)))?;
        Ok(())
    }

    pub fn insert_signal_log(&self, entry: &SignalLogInsert<'_>) -> Result<(), PolybotError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO signal_log (signal_id, timestamp, wallet_address, market_id, confidence, secret_level, category, side, disposition, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'))",
            rusqlite::params![
                entry.signal_id,
                entry.timestamp,
                entry.wallet_address,
                entry.market_id,
                entry.confidence,
                entry.secret_level,
                entry.category,
                entry.side,
                entry.disposition
            ],
        ).map_err(|e| PolybotError::State(format!("Failed to insert signal log: {}", e)))?;
        Ok(())
    }

    /// Persist signal to the PRD-compliant `signals` table.
    pub fn insert_signal(
        &self,
        signal: &polybot_common::types::Signal,
        source: &str,
        outcome: &str,
        status: &str,
    ) -> Result<(), PolybotError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO signals (id, source, tx_hash, received_at, target_wallet, market_id, token_id, side, direction, outcome, target_price, target_size, confidence, secret_level, category, status)
             VALUES (?1, ?2, ?3, datetime('now'), ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                signal.signal_id,
                source,
                signal.tx_hash.as_deref(),
                signal.wallet_address,
                signal.market_id,
                signal.token_id.as_deref().unwrap_or(""),
                format!("{:?}", signal.side),
                format!("{:?}", signal.direction),
                outcome,
                signal.target_price.map(|v| v.to_string()).unwrap_or_else(|| Decimal::ZERO.to_string()),
                signal.target_size_usdc.map(|v| v.to_string()).unwrap_or_else(|| Decimal::ZERO.to_string()),
                signal.confidence,
                signal.secret_level,
                signal.category.to_string(),
                status,
            ],
        ).map_err(|e| PolybotError::State(format!("Failed to insert signal: {}", e)))?;
        Ok(())
    }

    /// Anti-duplication rule: find which wallet owns an open position for a given token_id.
    pub fn get_open_position_owner_by_token(
        &self,
        token_id: &str,
    ) -> Result<Option<String>, PolybotError> {
        use rusqlite::OptionalExtension as _;
        self.conn
            .query_row(
                "SELECT p.owned_by_wallet
                 FROM positions p
                 JOIN signals s
                   ON s.market_id = p.market_id
                  AND LOWER(s.side) = LOWER(p.side)
                  AND s.target_wallet = p.owned_by_wallet
                 WHERE p.owned_by_wallet IS NOT NULL
                   AND p.status IN ('Open', 'open')
                   AND s.token_id = ?1
                 LIMIT 1",
                [token_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| {
                PolybotError::State(format!("Failed to query position owner by token: {}", e))
            })
    }

    pub fn update_signal_status(&self, signal_id: &str, status: &str) -> Result<(), PolybotError> {
        self.conn
            .execute(
                "UPDATE signals SET status = ?2 WHERE id = ?1",
                [signal_id, status],
            )
            .map_err(|e| PolybotError::State(format!("Failed to update signal status: {}", e)))?;
        Ok(())
    }

    pub fn get_signal(&self, signal_id: &str) -> Result<Option<PersistedSignalRow>, PolybotError> {
        use rusqlite::OptionalExtension as _;
        self.conn.query_row(
            "SELECT id, source, tx_hash, received_at, target_wallet, market_id, token_id, side, direction, outcome, target_price, target_size, confidence, secret_level, category, status FROM signals WHERE id = ?1",
            [signal_id],
            |row| {
                Ok(PersistedSignalRow {
                    id: row.get(0)?,
                    source: row.get(1)?,
                    tx_hash: row.get(2)?,
                    received_at: row.get(3)?,
                    target_wallet: row.get(4)?,
                    market_id: row.get(5)?,
                    token_id: row.get(6)?,
                    side: row.get(7)?,
                    direction: row.get(8)?,
                    outcome: row.get(9)?,
                    target_price: Decimal::from_str(&row.get::<_, String>(10)?).unwrap_or(Decimal::ZERO),
                    target_size: Decimal::from_str(&row.get::<_, String>(11)?).unwrap_or(Decimal::ZERO),
                    confidence: row.get(12)?,
                    secret_level: row.get(13)?,
                    category: row.get(14)?,
                    status: row.get(15)?,
                })
            },
        ).optional().map_err(|e| PolybotError::State(format!("Failed to get signal: {}", e)))
    }

    pub fn get_trade_count(&self) -> Result<u64, PolybotError> {
        let count: u64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM trades", [], |row| row.get(0))
            .map_err(|e| PolybotError::State(format!("Failed to count trades: {}", e)))?;
        Ok(count)
    }

    pub fn upsert_position(
        &self,
        position: &Position,
        current_price: Option<Decimal>,
        unrealized_pnl: Option<Decimal>,
        owned_by_wallet: Option<&str>,
    ) -> Result<(), PolybotError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO positions (id, market_id, side, entry_price, current_size, average_price, opened_at, status, category, current_price, unrealized_pnl, owned_by_wallet, last_updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, CURRENT_TIMESTAMP)",
            rusqlite::params![
                position.id,
                position.market_id,
                format!("{:?}", position.side),
                position.entry_price.to_string(),
                position.current_size.to_string(),
                position.average_price.to_string(),
                position.opened_at.to_rfc3339(),
                format!("{:?}", position.status),
                position.category.to_string(),
                current_price.map(|value| value.to_string()),
                unrealized_pnl.map(|value| value.to_string()),
                owned_by_wallet,
            ],
        ).map_err(|e| PolybotError::State(format!("Failed to upsert position: {}", e)))?;
        Ok(())
    }

    pub fn remove_position(&self, position_id: &str) -> Result<(), PolybotError> {
        self.conn
            .execute("DELETE FROM positions WHERE id = ?1", [position_id])
            .map_err(|e| PolybotError::State(format!("Failed to delete position: {}", e)))?;
        Ok(())
    }

    pub fn upsert_copied_lot(&self, row: &CopiedLotRow) -> Result<(), PolybotError> {
        let side = Self::canonicalize_copied_lot_side(&row.side);

        let updated_rows = self
            .conn
            .execute(
                "UPDATE copied_lots
                 SET side = ?4,
                     current_size = ?5,
                     average_price = ?6,
                     updated_at = ?7,
                     last_signal_id = ?8,
                     last_tx_hash = ?9
                 WHERE source_wallet = ?1 AND market_id = ?2 AND UPPER(side) = ?3",
                rusqlite::params![
                    row.source_wallet.to_lowercase(),
                    row.market_id,
                    side,
                    side,
                    row.current_size.to_string(),
                    row.average_price.to_string(),
                    row.updated_at,
                    row.last_signal_id,
                    row.last_tx_hash,
                ],
            )
            .map_err(|e| PolybotError::State(format!("Failed to update copied lot: {}", e)))?;

        if updated_rows == 0 {
            self.conn
                .execute(
                    "INSERT INTO copied_lots (id, source_wallet, market_id, side, current_size, average_price, opened_at, updated_at, last_signal_id, last_tx_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        row.id,
                        row.source_wallet.to_lowercase(),
                        row.market_id,
                        side,
                        row.current_size.to_string(),
                        row.average_price.to_string(),
                        row.opened_at,
                        row.updated_at,
                        row.last_signal_id,
                        row.last_tx_hash,
                    ],
                )
                .map_err(|e| PolybotError::State(format!("Failed to insert copied lot: {}", e)))?;
        }

        Ok(())
    }

    pub fn get_copied_lot(
        &self,
        source_wallet: &str,
        market_id: &str,
        side: &str,
    ) -> Result<Option<CopiedLotRow>, PolybotError> {
        use rusqlite::OptionalExtension as _;

        let side = Self::canonicalize_copied_lot_side(side);

        self.conn
            .query_row(
                "SELECT id, source_wallet, market_id, side, current_size, average_price, opened_at, updated_at, last_signal_id, last_tx_hash
                 FROM copied_lots WHERE source_wallet = ?1 AND market_id = ?2 AND UPPER(side) = ?3",
                rusqlite::params![source_wallet.to_lowercase(), market_id, side],
                |row| {
                    Ok(CopiedLotRow {
                        id: row.get(0)?,
                        source_wallet: row.get(1)?,
                        market_id: row.get(2)?,
                        side: row.get(3)?,
                        current_size: Decimal::from_str(&row.get::<_, String>(4)?)
                            .unwrap_or(Decimal::ZERO),
                        average_price: Decimal::from_str(&row.get::<_, String>(5)?)
                            .unwrap_or(Decimal::ZERO),
                        opened_at: row.get(6)?,
                        updated_at: row.get(7)?,
                        last_signal_id: row.get(8)?,
                        last_tx_hash: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(|e| PolybotError::State(format!("Failed to get copied lot: {}", e)))
    }

    pub fn delete_copied_lot(
        &self,
        source_wallet: &str,
        market_id: &str,
        side: &str,
    ) -> Result<(), PolybotError> {
        let side = Self::canonicalize_copied_lot_side(side);

        self.conn
            .execute(
                "DELETE FROM copied_lots WHERE source_wallet = ?1 AND market_id = ?2 AND UPPER(side) = ?3",
                rusqlite::params![source_wallet.to_lowercase(), market_id, side],
            )
            .map_err(|e| PolybotError::State(format!("Failed to delete copied lot: {}", e)))?;
        Ok(())
    }

    pub fn list_open_positions(&self) -> Result<Vec<PersistedPositionRow>, PolybotError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, market_id, side, entry_price, current_size, average_price, opened_at, status, category, current_price, unrealized_pnl, owned_by_wallet, last_updated
             FROM positions WHERE status = 'Open' OR status = 'open' ORDER BY opened_at ASC",
        ).map_err(|e| PolybotError::State(format!("Failed to prepare open positions query: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                let side_raw: String = row.get(2)?;
                let status_raw: String = row.get(7)?;
                let category_raw: String = row.get(8)?;
                let opened_at: String = row.get(6)?;
                let position = Position {
                    id: row.get(0)?,
                    market_id: row.get(1)?,
                    side: match side_raw.as_str() {
                        "Yes" | "YES" => Side::Yes,
                        _ => Side::No,
                    },
                    entry_price: Decimal::from_str(&row.get::<_, String>(3)?)
                        .unwrap_or(Decimal::ZERO),
                    current_size: Decimal::from_str(&row.get::<_, String>(4)?)
                        .unwrap_or(Decimal::ZERO),
                    average_price: Decimal::from_str(&row.get::<_, String>(5)?)
                        .unwrap_or(Decimal::ZERO),
                    opened_at: chrono::DateTime::parse_from_rfc3339(&opened_at)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    status: match status_raw.as_str() {
                        "Closed" | "closed" => PositionStatus::Closed,
                        "Ghost" | "ghost" => PositionStatus::Ghost,
                        _ => PositionStatus::Open,
                    },
                    category: Category::try_from(category_raw.as_str()).unwrap_or(Category::Other),
                };
                Ok(PersistedPositionRow {
                    position,
                    current_price: row
                        .get::<_, Option<String>>(9)?
                        .and_then(|value| Decimal::from_str(&value).ok()),
                    unrealized_pnl: row
                        .get::<_, Option<String>>(10)?
                        .and_then(|value| Decimal::from_str(&value).ok()),
                    owned_by_wallet: row.get(11)?,
                    last_updated: row.get(12)?,
                })
            })
            .map_err(|e| PolybotError::State(format!("Failed to query open positions: {}", e)))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| PolybotError::State(format!("Failed to read open positions: {}", e)))
    }

    pub fn get_market_metadata(
        &self,
        condition_id: &str,
    ) -> Result<Option<MarketMetadataRow>, PolybotError> {
        use rusqlite::OptionalExtension as _;
        self.conn
            .query_row(
                "SELECT condition_id, question, slug, icon, resolved, fetched_at FROM market_metadata WHERE condition_id = ?1",
                [condition_id.to_lowercase()],
                |row| {
                    Ok(MarketMetadataRow {
                        condition_id: row.get(0)?,
                        question: row.get(1)?,
                        slug: row.get(2)?,
                        icon: row.get(3)?,
                        resolved: row.get::<_, i64>(4)? == 1,
                        fetched_at: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(|e| PolybotError::State(format!("Failed to get market metadata: {}", e)))
    }

    pub fn upsert_market_metadata(&self, row: &MarketMetadataRow) -> Result<(), PolybotError> {
        self.conn.execute(
            "INSERT INTO market_metadata (condition_id, question, slug, icon, resolved, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5, CURRENT_TIMESTAMP)
             ON CONFLICT(condition_id) DO UPDATE SET
                question = excluded.question,
                slug = excluded.slug,
                icon = excluded.icon,
                resolved = excluded.resolved,
                fetched_at = CURRENT_TIMESTAMP",
            rusqlite::params![
                row.condition_id.to_lowercase(),
                row.question,
                row.slug,
                row.icon,
                if row.resolved { 1 } else { 0 },
            ],
        ).map_err(|e| PolybotError::State(format!("Failed to upsert market metadata: {}", e)))?;
        Ok(())
    }

    pub fn lookup_signal_wallet(&self, signal_id: &str) -> Result<Option<String>, PolybotError> {
        use rusqlite::OptionalExtension as _;
        self.conn
            .query_row(
                "SELECT wallet_address FROM signal_log WHERE signal_id = ?1",
                [signal_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| PolybotError::State(format!("Failed to lookup signal wallet: {}", e)))
    }

    pub fn set_config(&self, key: &str, value: &str) -> Result<(), PolybotError> {
        self.conn
            .execute(
                "INSERT INTO config (key, value, updated_at) VALUES (?1, ?2, CURRENT_TIMESTAMP)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = CURRENT_TIMESTAMP",
                rusqlite::params![key, value],
            )
            .map_err(|e| PolybotError::State(format!("Failed to upsert config: {}", e)))?;
        Ok(())
    }

    pub fn get_config(&self, key: &str) -> Result<Option<String>, PolybotError> {
        use rusqlite::OptionalExtension as _;
        self.conn
            .query_row("SELECT value FROM config WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| PolybotError::State(format!("Failed to load config: {}", e)))
    }

    pub fn upsert_daily_stats(&self, stats: &DailyStatsRow) -> Result<(), PolybotError> {
        self.conn.execute(
            "INSERT INTO daily_stats (date, starting_balance, realized_pnl, unrealized_pnl, volume_traded, trades_placed, trades_filled, trades_rejected, drawdown_pct, paused_at, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(date) DO UPDATE SET
                starting_balance = excluded.starting_balance,
                realized_pnl = excluded.realized_pnl,
                unrealized_pnl = excluded.unrealized_pnl,
                volume_traded = excluded.volume_traded,
                trades_placed = excluded.trades_placed,
                trades_filled = excluded.trades_filled,
                trades_rejected = excluded.trades_rejected,
                drawdown_pct = excluded.drawdown_pct,
                paused_at = excluded.paused_at,
                notes = excluded.notes",
            rusqlite::params![
                stats.date,
                stats.starting_balance.to_string(),
                stats.realized_pnl.to_string(),
                stats.unrealized_pnl.to_string(),
                stats.volume_traded.to_string(),
                stats.trades_placed,
                stats.trades_filled,
                stats.trades_rejected,
                stats.drawdown_pct.to_string(),
                stats.paused_at,
                stats.notes,
            ],
        ).map_err(|e| PolybotError::State(format!("Failed to upsert daily stats: {}", e)))?;
        Ok(())
    }

    pub fn get_daily_stats(&self, date: &str) -> Result<Option<DailyStatsRow>, PolybotError> {
        use rusqlite::OptionalExtension as _;
        self.conn.query_row(
            "SELECT date, starting_balance, realized_pnl, unrealized_pnl, volume_traded, trades_placed, trades_filled, trades_rejected, drawdown_pct, paused_at, notes FROM daily_stats WHERE date = ?1",
            [date],
            |row| {
                Ok(DailyStatsRow {
                    date: row.get(0)?,
                    starting_balance: Decimal::from_str(&row.get::<_, String>(1)?).unwrap_or(Decimal::ZERO),
                    realized_pnl: Decimal::from_str(&row.get::<_, String>(2)?).unwrap_or(Decimal::ZERO),
                    unrealized_pnl: Decimal::from_str(&row.get::<_, String>(3)?).unwrap_or(Decimal::ZERO),
                    volume_traded: Decimal::from_str(&row.get::<_, String>(4)?).unwrap_or(Decimal::ZERO),
                    trades_placed: row.get(5)?,
                    trades_filled: row.get(6)?,
                    trades_rejected: row.get(7)?,
                    drawdown_pct: Decimal::from_str(&row.get::<_, String>(8)?).unwrap_or(Decimal::ZERO),
                    paused_at: row.get(9)?,
                    notes: row.get(10)?,
                })
            },
        ).optional().map_err(|e| PolybotError::State(format!("Failed to load daily stats: {}", e)))
    }

    /// Retrieve the last N days of daily stats, ordered by date descending.
    pub fn get_recent_daily_stats(&self, limit: usize) -> Result<Vec<DailyStatsRow>, PolybotError> {
        let mut stmt = self.conn.prepare(
            "SELECT date, starting_balance, realized_pnl, unrealized_pnl, volume_traded, trades_placed, trades_filled, trades_rejected, drawdown_pct, paused_at, notes FROM daily_stats ORDER BY date DESC LIMIT ?1"
        ).map_err(|e| PolybotError::State(format!("Failed to prepare recent stats: {}", e)))?;
        let rows = stmt
            .query_map([limit], |row| {
                Ok(DailyStatsRow {
                    date: row.get(0)?,
                    starting_balance: Decimal::from_str(&row.get::<_, String>(1)?)
                        .unwrap_or(Decimal::ZERO),
                    realized_pnl: Decimal::from_str(&row.get::<_, String>(2)?)
                        .unwrap_or(Decimal::ZERO),
                    unrealized_pnl: Decimal::from_str(&row.get::<_, String>(3)?)
                        .unwrap_or(Decimal::ZERO),
                    volume_traded: Decimal::from_str(&row.get::<_, String>(4)?)
                        .unwrap_or(Decimal::ZERO),
                    trades_placed: row.get(5)?,
                    trades_filled: row.get(6)?,
                    trades_rejected: row.get(7)?,
                    drawdown_pct: Decimal::from_str(&row.get::<_, String>(8)?)
                        .unwrap_or(Decimal::ZERO),
                    paused_at: row.get(9)?,
                    notes: row.get(10)?,
                })
            })
            .map_err(|e| PolybotError::State(format!("Failed to query recent stats: {}", e)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| PolybotError::State(format!("Row error: {}", e)))?);
        }
        Ok(out)
    }

    pub fn upsert_target(
        &self,
        wallet_address: &str,
        label: Option<&str>,
        categories: &[Category],
        score: Option<Decimal>,
    ) -> Result<(), PolybotError> {
        let categories_json = serde_json::to_string(categories).map_err(|e| {
            PolybotError::State(format!("Failed to serialize target categories: {}", e))
        })?;
        self.conn
            .execute(
                "INSERT INTO targets (wallet_address, label, active, categories, score)
             VALUES (?1, ?2, 1, ?3, ?4)
             ON CONFLICT(wallet_address) DO UPDATE SET
                label = excluded.label,
                active = 1,
                categories = excluded.categories,
                score = excluded.score",
                rusqlite::params![
                    wallet_address.to_lowercase(),
                    label,
                    categories_json,
                    score.map(|value| value.to_string())
                ],
            )
            .map_err(|e| PolybotError::State(format!("Failed to upsert target wallet: {}", e)))?;
        Ok(())
    }

    pub fn list_active_targets(&self) -> Result<Vec<TargetRow>, PolybotError> {
        let mut stmt = self.conn.prepare(
            "SELECT wallet_address, label, categories, score, active FROM targets WHERE active = 1 ORDER BY added_at ASC",
        ).map_err(|e| PolybotError::State(format!("Failed to prepare targets query: {}", e)))?;

        let rows = stmt
            .query_map([], |row| {
                let categories_json: String = row.get(2)?;
                let categories =
                    serde_json::from_str::<Vec<Category>>(&categories_json).unwrap_or_default();
                let score = row
                    .get::<_, Option<String>>(3)?
                    .and_then(|value| Decimal::from_str(&value).ok());
                Ok(TargetRow {
                    wallet_address: row.get::<_, String>(0)?.to_lowercase(),
                    label: row.get(1)?,
                    categories,
                    score,
                    active: row.get::<_, i64>(4)? == 1,
                })
            })
            .map_err(|e| PolybotError::State(format!("Failed to query targets: {}", e)))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| PolybotError::State(format!("Failed to read targets: {}", e)))
    }

    pub fn latest_trades(&self, limit: usize) -> Result<Vec<RecentTradeRow>, PolybotError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, signal_id, source_wallet, market_id, COALESCE(category, ''), side, direction, status, size_usd, placed_at, simulated
                  FROM trades ORDER BY placed_at DESC LIMIT ?1",
            )
            .map_err(|e| PolybotError::State(format!("Failed to prepare trades query: {}", e)))?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(RecentTradeRow {
                    id: row.get(0)?,
                    signal_id: row.get(1)?,
                    source_wallet: row.get(2)?,
                    market_id: row.get(3)?,
                    category: row.get(4)?,
                    side: row.get(5)?,
                    direction: row.get(6)?,
                    status: row.get(7)?,
                    size_usd: Decimal::from_str(&row.get::<_, String>(8)?).unwrap_or(Decimal::ZERO),
                    placed_at: row.get(9)?,
                    simulated: row.get::<_, i64>(10)? == 1,
                })
            })
            .map_err(|e| PolybotError::State(format!("Failed to query trades: {}", e)))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| PolybotError::State(format!("Failed to read trades: {}", e)))
    }

    pub fn deactivate_target(&self, wallet_address: &str) -> Result<(), PolybotError> {
        self.conn
            .execute(
                "UPDATE targets SET active = 0 WHERE wallet_address = ?1",
                [wallet_address.to_lowercase()],
            )
            .map_err(|e| {
                PolybotError::State(format!("Failed to deactivate target wallet: {}", e))
            })?;
        Ok(())
    }

    pub fn latest_signals(&self, limit: usize) -> Result<Vec<SignalLogEntry>, PolybotError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT signal_id, timestamp, wallet_address, market_id, confidence, secret_level, category, side, disposition, received_at
                 FROM signal_log
                 ORDER BY received_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| PolybotError::State(format!("Failed to prepare signal query: {}", e)))?;

        let rows = stmt
            .query_map([limit as i64], |row| {
                Ok(SignalLogEntry {
                    signal_id: row.get(0)?,
                    timestamp: row.get(1)?,
                    wallet_address: row.get(2)?,
                    market_id: row.get(3)?,
                    confidence: row.get(4)?,
                    secret_level: row.get(5)?,
                    category: row.get(6)?,
                    side: row.get(7)?,
                    disposition: row.get(8)?,
                    received_at: row.get(9)?,
                })
            })
            .map_err(|e| PolybotError::State(format!("Failed to query signals: {}", e)))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| PolybotError::State(format!("Failed to read signals: {}", e)))
    }

    pub fn insert_transaction(
        &self,
        rec: &polybot_common::types::TransactionRecord,
    ) -> Result<(), PolybotError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO transactions
                (transaction_id, trade_id, type, state, submitted_at, confirmed_at, transaction_hash, error_msg)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                rec.transaction_id,
                rec.trade_id,
                rec.kind.as_sqlite_str(),
                rec.state.as_sqlite_str(),
                rec.submitted_at.to_rfc3339(),
                rec.confirmed_at.map(|t| t.to_rfc3339()),
                rec.transaction_hash,
                rec.error_msg,
            ],
        )
        .map_err(|e| PolybotError::State(format!("Failed to insert transaction: {}", e)))?;
        Ok(())
    }

    pub fn update_transaction_state(
        &self,
        transaction_id: &str,
        state: polybot_common::types::TransactionState,
        transaction_hash: Option<&str>,
        error_msg: Option<&str>,
    ) -> Result<(), PolybotError> {
        let confirmed_at = if state.is_terminal() {
            Some(chrono::Utc::now().to_rfc3339())
        } else {
            None
        };
        self.conn
            .execute(
                "UPDATE transactions
             SET state = ?2,
                 confirmed_at = COALESCE(?3, confirmed_at),
                 transaction_hash = COALESCE(?4, transaction_hash),
                 error_msg = COALESCE(?5, error_msg)
             WHERE transaction_id = ?1",
                rusqlite::params![
                    transaction_id,
                    state.as_sqlite_str(),
                    confirmed_at,
                    transaction_hash,
                    error_msg,
                ],
            )
            .map_err(|e| {
                PolybotError::State(format!("Failed to update transaction state: {}", e))
            })?;
        Ok(())
    }

    pub fn get_transaction(
        &self,
        transaction_id: &str,
    ) -> Result<Option<polybot_common::types::TransactionRecord>, PolybotError> {
        use rusqlite::OptionalExtension as _;
        self.conn
            .query_row(
                "SELECT transaction_id, trade_id, type, state, submitted_at, confirmed_at, transaction_hash, error_msg
                 FROM transactions WHERE transaction_id = ?1",
                [transaction_id],
                Self::row_to_transaction_record,
            )
            .optional()
            .map_err(|e| PolybotError::State(format!("Failed to query transaction: {}", e)))
    }

    pub fn list_non_terminal_transactions(
        &self,
    ) -> Result<Vec<polybot_common::types::TransactionRecord>, PolybotError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT transaction_id, trade_id, type, state, submitted_at, confirmed_at, transaction_hash, error_msg
                 FROM transactions
                 WHERE state NOT IN ('STATE_SUCCESS', 'STATE_FAILED')
                 ORDER BY submitted_at ASC",
            )
            .map_err(|e| PolybotError::State(format!("Failed to prepare list-tx query: {}", e)))?;
        let rows = stmt
            .query_map([], Self::row_to_transaction_record)
            .map_err(|e| PolybotError::State(format!("Failed to execute list-tx query: {}", e)))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| PolybotError::State(format!("Failed to collect tx rows: {}", e)))
    }

    pub fn latest_transactions(
        &self,
        limit: usize,
    ) -> Result<Vec<polybot_common::types::TransactionRecord>, PolybotError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT transaction_id, trade_id, type, state, submitted_at, confirmed_at, transaction_hash, error_msg
                 FROM transactions
                 ORDER BY submitted_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| {
                PolybotError::State(format!("Failed to prepare latest transactions: {}", e))
            })?;
        let rows = stmt
            .query_map([limit as i64], Self::row_to_transaction_record)
            .map_err(|e| {
                PolybotError::State(format!("Failed to query latest transactions: {}", e))
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
            PolybotError::State(format!("Failed to collect latest transactions: {}", e))
        })
    }

    fn row_to_transaction_record(
        row: &rusqlite::Row<'_>,
    ) -> rusqlite::Result<polybot_common::types::TransactionRecord> {
        use polybot_common::types::{TransactionKind, TransactionRecord, TransactionState};
        let kind_str: String = row.get(2)?;
        let state_str: String = row.get(3)?;
        let kind = match kind_str.as_str() {
            "order" => TransactionKind::Order,
            "cancel" => TransactionKind::Cancel,
            "wrap" => TransactionKind::Wrap,
            "approve" => TransactionKind::Approve,
            "redeem" => TransactionKind::Redeem,
            "deploy" => TransactionKind::Deploy,
            other => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    format!("unknown transaction kind: {}", other).into(),
                ));
            }
        };
        let state = match state_str.as_str() {
            "STATE_NEW" => TransactionState::New,
            "STATE_PENDING" => TransactionState::Pending,
            "STATE_SUBMITTED" => TransactionState::Submitted,
            "STATE_SUCCESS" => TransactionState::Success,
            "STATE_FAILED" => TransactionState::Failed,
            other => {
                return Err(rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    format!("unknown transaction state: {}", other).into(),
                ));
            }
        };
        let submitted_at_str: String = row.get(4)?;
        let submitted_at = chrono::DateTime::parse_from_rfc3339(&submitted_at_str)
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?
            .with_timezone(&chrono::Utc);
        let confirmed_at_str: Option<String> = row.get(5)?;
        let confirmed_at = confirmed_at_str
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(&s).map(|d| d.with_timezone(&chrono::Utc))
            })
            .transpose()
            .map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
        Ok(TransactionRecord {
            transaction_id: row.get(0)?,
            trade_id: row.get(1)?,
            kind,
            state,
            submitted_at,
            confirmed_at,
            transaction_hash: row.get(6)?,
            error_msg: row.get(7)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use polybot_common::types::*;
    use rust_decimal_macros::dec;

    #[test]
    fn open_in_memory() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.get_trade_count().unwrap(), 0);
    }

    #[test]
    fn insert_and_count_trade() {
        let store = SqliteStore::open_in_memory().unwrap();
        let trade = Trade {
            id: "t1".to_string(),
            signal_id: "s1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "m1".to_string(),
            category: Category::Politics,
            side: Side::Yes,
            direction: TradeDirection::Buy,
            price: dec!(0.65),
            size: dec!(100),
            size_usd: dec!(65),
            filled_size: dec!(100),
            order_type: OrderType::Limit,
            status: TradeStatus::Filled,
            placed_at: chrono::Utc::now(),
            filled_at: Some(chrono::Utc::now()),
            simulated: true,
            transaction_id: None,
            transaction_hash: None,
            relayer_state: None,
            taker_fee_bps: 0,
            fee_paid_usdc: Decimal::ZERO,
            rebate_usdc: Decimal::ZERO,
            retry_count: 0,
            error_msg: None,
        };
        store.insert_trade(&trade).unwrap();
        assert_eq!(store.get_trade_count().unwrap(), 1);
    }

    #[test]
    fn insert_signal_log() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .insert_signal_log(&SignalLogInsert {
                signal_id: "sig-1",
                timestamp: "2026-04-14T12:00:00Z",
                wallet_address: "0xabc",
                market_id: "m1",
                confidence: 7,
                secret_level: 6,
                category: "politics",
                side: "YES",
                disposition: "execute",
            })
            .unwrap();
    }

    #[test]
    fn latest_signals_returns_rows() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .insert_signal_log(&SignalLogInsert {
                signal_id: "sig-1",
                timestamp: "2026-04-14T12:00:00Z",
                wallet_address: "0xabc",
                market_id: "m1",
                confidence: 7,
                secret_level: 6,
                category: "politics",
                side: "YES",
                disposition: "execute",
            })
            .unwrap();

        let signals = store.latest_signals(10).unwrap();
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_id, "sig-1");
    }

    #[test]
    fn upsert_and_list_open_positions_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let position = Position {
            id: "pos-1".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            entry_price: dec!(0.60),
            current_size: dec!(100),
            average_price: dec!(0.60),
            opened_at: chrono::Utc::now(),
            status: PositionStatus::Open,
            category: Category::Politics,
        };

        store
            .upsert_position(&position, Some(dec!(0.65)), Some(dec!(5)), Some("0xabc"))
            .unwrap();

        let positions = store.list_open_positions().unwrap();
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].position.market_id, "market-1");
        assert_eq!(positions[0].current_price, Some(dec!(0.65)));
        assert_eq!(positions[0].unrealized_pnl, Some(dec!(5)));
        assert_eq!(positions[0].owned_by_wallet.as_deref(), Some("0xabc"));
    }

    #[test]
    fn get_open_position_owner_by_token_uses_persisted_signal_token_mapping() {
        let store = SqliteStore::open_in_memory().unwrap();
        let signal = Signal {
            signal_id: "sig-1".to_string(),
            timestamp: "2026-04-17T10:00:00Z".to_string(),
            wallet_address: "0xabc".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            direction: TradeDirection::Buy,
            confidence: 7,
            secret_level: 6,
            category: Category::Politics,
            source: SignalSource::Polling,
            tx_hash: Some(
                "0x1234567890123456789012345678901234567890123456789012345678901234".to_string(),
            ),
            token_id: Some("token-1".to_string()),
            target_price: Some(dec!(0.60)),
            target_size_usdc: Some(dec!(60)),
            target_size_tokens: None,
            resolved: false,
            redeemable: false,
            suggested_size_usdc: None,
            fee_schedule: None,
            scanner_version: "1.0.0".to_string(),
        };
        store
            .insert_signal(&signal, "polling", "YES", "executed")
            .unwrap();

        let position = Position {
            id: "pos-1".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            entry_price: dec!(0.60),
            current_size: dec!(100),
            average_price: dec!(0.60),
            opened_at: chrono::Utc::now(),
            status: PositionStatus::Open,
            category: Category::Politics,
        };
        store
            .upsert_position(&position, Some(dec!(0.60)), Some(dec!(0)), Some("0xabc"))
            .unwrap();

        assert_eq!(
            store
                .get_open_position_owner_by_token("token-1")
                .unwrap()
                .as_deref(),
            Some("0xabc")
        );
    }

    #[test]
    fn config_and_daily_stats_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .set_config("last_reconciliation_at", "2026-04-17T10:00:00Z")
            .unwrap();
        assert_eq!(
            store
                .get_config("last_reconciliation_at")
                .unwrap()
                .as_deref(),
            Some("2026-04-17T10:00:00Z")
        );

        let stats = DailyStatsRow {
            date: "2026-04-17".to_string(),
            starting_balance: dec!(1000),
            realized_pnl: dec!(25),
            unrealized_pnl: dec!(10),
            volume_traded: dec!(150),
            trades_placed: 3,
            trades_filled: 2,
            trades_rejected: 1,
            drawdown_pct: dec!(0.05),
            paused_at: None,
            notes: Some("healthy".to_string()),
        };

        store.upsert_daily_stats(&stats).unwrap();
        let loaded = store.get_daily_stats("2026-04-17").unwrap().unwrap();
        assert_eq!(loaded.realized_pnl, dec!(25));
        assert_eq!(loaded.trades_filled, 2);
        assert_eq!(loaded.notes.as_deref(), Some("healthy"));
    }

    #[test]
    fn copied_lot_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let lot = CopiedLotRow {
            id: "lot-1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            side: "YES".to_string(),
            current_size: dec!(10),
            average_price: dec!(0.55),
            opened_at: "2026-04-24T12:00:00Z".to_string(),
            updated_at: "2026-04-24T12:00:00Z".to_string(),
            last_signal_id: Some("sig-1".to_string()),
            last_tx_hash: None,
        };

        store.upsert_copied_lot(&lot).unwrap();
        let loaded = store
            .get_copied_lot(&lot.source_wallet, &lot.market_id, &lot.side)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.current_size, dec!(10));
    }

    #[test]
    fn canonical_side_casing_round_trips_through_sqlite() {
        let store = SqliteStore::open_in_memory().unwrap();
        let lot = CopiedLotRow {
            id: "lot-1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            side: "Yes".to_string(),
            current_size: dec!(10),
            average_price: dec!(0.55),
            opened_at: "2026-04-24T12:00:00Z".to_string(),
            updated_at: "2026-04-24T12:00:00Z".to_string(),
            last_signal_id: Some("sig-1".to_string()),
            last_tx_hash: None,
        };

        store.upsert_copied_lot(&lot).unwrap();

        let loaded = store
            .get_copied_lot(&lot.source_wallet, &lot.market_id, "YES")
            .unwrap()
            .unwrap();

        assert_eq!(loaded.side, "YES");

        store
            .delete_copied_lot(&lot.source_wallet, &lot.market_id, "yes")
            .unwrap();
        assert!(store
            .get_copied_lot(&lot.source_wallet, &lot.market_id, "YES")
            .unwrap()
            .is_none());
    }

    #[test]
    fn upsert_conflict_preserves_id_and_opened_at() {
        let store = SqliteStore::open_in_memory().unwrap();
        let original = CopiedLotRow {
            id: "lot-1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            side: "YES".to_string(),
            current_size: dec!(10),
            average_price: dec!(0.55),
            opened_at: "2026-04-24T12:00:00Z".to_string(),
            updated_at: "2026-04-24T12:00:00Z".to_string(),
            last_signal_id: Some("sig-1".to_string()),
            last_tx_hash: None,
        };
        let replacement = CopiedLotRow {
            id: "lot-2".to_string(),
            source_wallet: original.source_wallet.clone(),
            market_id: original.market_id.clone(),
            side: "Yes".to_string(),
            current_size: dec!(7),
            average_price: dec!(0.60),
            opened_at: "2026-04-25T13:00:00Z".to_string(),
            updated_at: "2026-04-25T13:00:00Z".to_string(),
            last_signal_id: Some("sig-2".to_string()),
            last_tx_hash: Some("0xabc".to_string()),
        };

        store.upsert_copied_lot(&original).unwrap();
        store.upsert_copied_lot(&replacement).unwrap();

        let loaded = store
            .get_copied_lot(&original.source_wallet, &original.market_id, &original.side)
            .unwrap()
            .unwrap();

        assert_eq!(loaded.id, original.id);
        assert_eq!(loaded.opened_at, original.opened_at);
        assert_eq!(loaded.current_size, replacement.current_size);
        assert_eq!(loaded.average_price, replacement.average_price);
        assert_eq!(loaded.updated_at, replacement.updated_at);
        assert_eq!(loaded.last_signal_id, replacement.last_signal_id);
        assert_eq!(loaded.last_tx_hash, replacement.last_tx_hash);
    }

    #[test]
    fn legacy_mixed_case_side_row_is_found_and_updated_by_canonical_path() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .conn
            .execute(
                "INSERT INTO copied_lots (id, source_wallet, market_id, side, current_size, average_price, opened_at, updated_at, last_signal_id, last_tx_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    "legacy-lot-1",
                    "0xabc123abc123abc123abc123abc123abc123abc1",
                    "market-1",
                    "Yes",
                    dec!(10).to_string(),
                    dec!(0.55).to_string(),
                    "2026-04-24T12:00:00Z",
                    "2026-04-24T12:00:00Z",
                    Some("sig-1".to_string()),
                    Option::<String>::None,
                ],
            )
            .unwrap();

        let loaded = store
            .get_copied_lot(
                "0xabc123abc123abc123abc123abc123abc123abc1",
                "market-1",
                "YES",
            )
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, "legacy-lot-1");

        let replacement = CopiedLotRow {
            id: "new-lot-id".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "market-1".to_string(),
            side: "YES".to_string(),
            current_size: dec!(7),
            average_price: dec!(0.60),
            opened_at: "2026-04-25T13:00:00Z".to_string(),
            updated_at: "2026-04-25T13:00:00Z".to_string(),
            last_signal_id: Some("sig-2".to_string()),
            last_tx_hash: Some("0xabc".to_string()),
        };

        store.upsert_copied_lot(&replacement).unwrap();

        let updated = store
            .get_copied_lot(&replacement.source_wallet, &replacement.market_id, "YES")
            .unwrap()
            .unwrap();
        let copied_lot_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM copied_lots", [], |row| row.get(0))
            .unwrap();

        assert_eq!(copied_lot_count, 1);
        assert_eq!(updated.id, "legacy-lot-1");
        assert_eq!(updated.side, "YES");
        assert_eq!(updated.opened_at, "2026-04-24T12:00:00Z");
        assert_eq!(updated.current_size, dec!(7));

        store
            .delete_copied_lot(&replacement.source_wallet, &replacement.market_id, "yes")
            .unwrap();
        assert!(store
            .get_copied_lot(&replacement.source_wallet, &replacement.market_id, "YES")
            .unwrap()
            .is_none());
    }

    #[test]
    fn active_targets_round_trip() {
        let store = SqliteStore::open_in_memory().unwrap();
        store
            .upsert_target(
                "0xabc123abc123abc123abc123abc123abc123abc1",
                Some("leader"),
                &[Category::Politics, Category::Crypto],
                Some(dec!(72.5)),
            )
            .unwrap();

        let targets = store.list_active_targets().unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].wallet_address,
            "0xabc123abc123abc123abc123abc123abc123abc1"
        );
        assert_eq!(
            targets[0].categories,
            vec![Category::Politics, Category::Crypto]
        );
        assert_eq!(targets[0].score, Some(dec!(72.5)));
    }

    #[test]
    fn latest_trades_returns_most_recent_rows_in_desc_order() {
        let store = SqliteStore::open_in_memory().unwrap();

        let trade1 = Trade {
            id: "t1".to_string(),
            signal_id: "s1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "m1".to_string(),
            category: Category::Politics,
            side: Side::Yes,
            direction: TradeDirection::Buy,
            price: dec!(0.55),
            size: dec!(10),
            size_usd: dec!(5.5),
            filled_size: dec!(10),
            order_type: OrderType::Limit,
            status: TradeStatus::Filled,
            placed_at: chrono::Utc::now() - chrono::Duration::seconds(30),
            filled_at: Some(chrono::Utc::now() - chrono::Duration::seconds(20)),
            simulated: true,
            transaction_id: None,
            transaction_hash: None,
            relayer_state: None,
            taker_fee_bps: 0,
            fee_paid_usdc: Decimal::ZERO,
            rebate_usdc: Decimal::ZERO,
            retry_count: 0,
            error_msg: None,
        };
        let trade2 = Trade {
            id: "t2".to_string(),
            signal_id: "s2".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "m2".to_string(),
            category: Category::Crypto,
            side: Side::No,
            direction: TradeDirection::Buy,
            price: dec!(0.65),
            size: dec!(10),
            size_usd: dec!(6.5),
            filled_size: dec!(10),
            order_type: OrderType::Fok,
            status: TradeStatus::Filled,
            placed_at: chrono::Utc::now(),
            filled_at: Some(chrono::Utc::now()),
            simulated: false,
            transaction_id: None,
            transaction_hash: None,
            relayer_state: None,
            taker_fee_bps: 0,
            fee_paid_usdc: Decimal::ZERO,
            rebate_usdc: Decimal::ZERO,
            retry_count: 0,
            error_msg: None,
        };

        store.insert_trade(&trade1).unwrap();
        store.insert_trade(&trade2).unwrap();

        let trades = store.latest_trades(10).unwrap();
        assert_eq!(trades.len(), 2);
        assert_eq!(trades[0].id, "t2");
        assert_eq!(trades[1].id, "t1");
    }

    #[test]
    fn get_signal_round_trip_preserves_direction() {
        let store = SqliteStore::open_in_memory().unwrap();
        let signal = Signal {
            signal_id: "sig-1".to_string(),
            timestamp: "2026-04-17T10:00:00Z".to_string(),
            wallet_address: "0xabc".to_string(),
            market_id: "market-1".to_string(),
            side: Side::Yes,
            direction: TradeDirection::Sell,
            confidence: 7,
            secret_level: 6,
            category: Category::Politics,
            source: SignalSource::Polling,
            tx_hash: None,
            token_id: Some("token-1".to_string()),
            target_price: Some(dec!(0.60)),
            target_size_usdc: Some(dec!(60)),
            target_size_tokens: None,
            resolved: false,
            redeemable: false,
            suggested_size_usdc: None,
            fee_schedule: None,
            scanner_version: "1.0.0".to_string(),
        };

        store
            .insert_signal(&signal, "polling", "YES", "executed")
            .unwrap();
        let persisted = store.get_signal("sig-1").unwrap().unwrap();

        assert_eq!(persisted.direction, "Sell");
    }

    #[test]
    fn latest_trades_round_trip_preserves_source_wallet_and_direction() {
        let store = SqliteStore::open_in_memory().unwrap();
        let trade = Trade {
            id: "t1".to_string(),
            signal_id: "s1".to_string(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".to_string(),
            market_id: "m1".to_string(),
            category: Category::Politics,
            side: Side::Yes,
            direction: TradeDirection::Sell,
            price: dec!(0.55),
            size: dec!(10),
            size_usd: dec!(5.5),
            filled_size: dec!(10),
            order_type: OrderType::Limit,
            status: TradeStatus::Filled,
            placed_at: chrono::Utc::now(),
            filled_at: Some(chrono::Utc::now()),
            simulated: true,
            transaction_id: None,
            transaction_hash: None,
            relayer_state: None,
            taker_fee_bps: 0,
            fee_paid_usdc: Decimal::ZERO,
            rebate_usdc: Decimal::ZERO,
            retry_count: 0,
            error_msg: None,
        };

        store.insert_trade(&trade).unwrap();
        let trades = store.latest_trades(10).unwrap();

        assert_eq!(trades[0].source_wallet, trade.source_wallet);
        assert_eq!(trades[0].direction, "Sell");
    }

    #[test]
    fn legacy_schema_rows_read_with_default_direction_and_source_wallet() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE signals (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                tx_hash TEXT UNIQUE,
                received_at TEXT NOT NULL,
                target_wallet TEXT NOT NULL,
                market_id TEXT NOT NULL,
                token_id TEXT NOT NULL,
                side TEXT NOT NULL,
                outcome TEXT NOT NULL,
                target_price TEXT NOT NULL,
                target_size TEXT NOT NULL,
                confidence INTEGER NOT NULL,
                secret_level INTEGER NOT NULL,
                category TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending'
            );
            CREATE TABLE trades (
                id TEXT PRIMARY KEY,
                signal_id TEXT NOT NULL,
                market_id TEXT NOT NULL,
                category TEXT,
                side TEXT NOT NULL,
                price TEXT NOT NULL,
                size TEXT NOT NULL,
                size_usd TEXT NOT NULL,
                filled_size TEXT NOT NULL,
                order_type TEXT NOT NULL,
                status TEXT NOT NULL,
                placed_at TEXT NOT NULL,
                filled_at TEXT,
                simulated INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO signals (id, source, tx_hash, received_at, target_wallet, market_id, token_id, side, outcome, target_price, target_size, confidence, secret_level, category, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                "sig-1",
                "polling",
                Option::<String>::None,
                "2026-04-17T10:00:00Z",
                "0xabc",
                "market-1",
                "token-1",
                "Yes",
                "YES",
                "0.60",
                "60",
                7,
                6,
                "politics",
                "executed",
            ],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO trades (id, signal_id, market_id, category, side, price, size, size_usd, filled_size, order_type, status, placed_at, filled_at, simulated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                "t1",
                "sig-1",
                "market-1",
                "politics",
                "Yes",
                "0.55",
                "10",
                "5.5",
                "10",
                "Limit",
                "Filled",
                "2026-04-17T10:00:00Z",
                Option::<String>::None,
                1,
            ],
        )
        .unwrap();

        let store = SqliteStore { conn };
        store.create_tables().unwrap();

        let signal = store.get_signal("sig-1").unwrap().unwrap();
        let trades = store.latest_trades(10).unwrap();

        assert_eq!(signal.direction, "Buy");
        assert_eq!(trades[0].source_wallet, "");
        assert_eq!(trades[0].direction, "Buy");
    }
}

#[cfg(test)]
mod migration_runner_tests {
    use super::SqliteStore;

    #[test]
    fn new_database_records_latest_migration_version() {
        let store = SqliteStore::open_in_memory().unwrap();
        let version: i64 = store
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version), -1) FROM schema_migrations",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(version >= 0, "migration runner did not record any version");
    }

    #[test]
    fn migrations_are_idempotent() {
        let store = SqliteStore::open_in_memory().unwrap();
        let initial: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        store
            .run_migrations_for_test()
            .expect("re-run should be no-op");
        let after: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(initial, after, "idempotent re-run should not add rows");
    }

    #[test]
    fn transactions_table_exists_after_migrations() {
        let store = SqliteStore::open_in_memory().unwrap();
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='transactions'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "transactions table missing");
    }

    #[test]
    fn transactions_table_has_expected_columns() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut stmt = store
            .conn
            .prepare("PRAGMA table_info(transactions)")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for required in [
            "transaction_id",
            "trade_id",
            "type",
            "state",
            "submitted_at",
            "confirmed_at",
            "transaction_hash",
            "error_msg",
        ] {
            assert!(
                cols.iter().any(|c| c == required),
                "missing column: {} (have {:?})",
                required,
                cols
            );
        }
    }

    #[test]
    fn signals_table_has_v2_fee_columns() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut stmt = store.conn.prepare("PRAGMA table_info(signals)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for required in ["taker_fee_bps", "maker_fee_bps", "rebate_bps"] {
            assert!(
                cols.iter().any(|c| c == required),
                "signals missing V2 column: {} (have {:?})",
                required,
                cols
            );
        }
    }

    #[test]
    fn trades_table_has_v2_relayer_columns() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut stmt = store.conn.prepare("PRAGMA table_info(trades)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for required in [
            "transaction_id",
            "transaction_hash",
            "relayer_state",
            "taker_fee_bps",
            "fee_paid_usdc",
            "rebate_usdc",
            "retry_count",
            "error_msg",
        ] {
            assert!(
                cols.iter().any(|c| c == required),
                "trades missing V2 column: {} (have {:?})",
                required,
                cols
            );
        }
    }

    #[test]
    fn daily_stats_has_v2_fee_columns() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut stmt = store
            .conn
            .prepare("PRAGMA table_info(daily_stats)")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for required in ["fees_paid", "rebates_earned"] {
            assert!(
                cols.iter().any(|c| c == required),
                "daily_stats missing V2 column: {} (have {:?})",
                required,
                cols
            );
        }
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let store = SqliteStore::open_in_memory().unwrap();
        let fk_on: i64 = store
            .conn
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fk_on, 1, "PRAGMA foreign_keys should be ON");

        // Write a transactions row that references a non-existent trade.
        let result = store.conn.execute(
            "INSERT INTO transactions
                (transaction_id, trade_id, type, state, submitted_at)
             VALUES ('tx-orphan', 'trade-does-not-exist', 'order', 'STATE_NEW',
                     '2026-04-24T00:00:00Z')",
            [],
        );
        assert!(
            result.is_err(),
            "FK constraint should reject orphan trade_id, got Ok({:?})",
            result
        );
    }
}

#[cfg(test)]
mod transactions_crud_tests {
    use super::SqliteStore;
    use chrono::Utc;
    use polybot_common::types::{TransactionKind, TransactionRecord, TransactionState};

    fn sample_record() -> TransactionRecord {
        TransactionRecord {
            transaction_id: "txn_abc123".into(),
            trade_id: None,
            kind: TransactionKind::Order,
            state: TransactionState::New,
            submitted_at: Utc::now(),
            confirmed_at: None,
            transaction_hash: None,
            error_msg: None,
        }
    }

    #[test]
    fn insert_and_get_transaction_roundtrip() {
        let store = SqliteStore::open_in_memory().unwrap();
        let rec = sample_record();
        store.insert_transaction(&rec).unwrap();
        let got = store
            .get_transaction(&rec.transaction_id)
            .unwrap()
            .expect("transaction should exist");
        assert_eq!(got.transaction_id, rec.transaction_id);
        assert_eq!(got.kind, TransactionKind::Order);
        assert_eq!(got.state, TransactionState::New);
    }

    #[test]
    fn update_transaction_state_promotes_to_terminal() {
        let store = SqliteStore::open_in_memory().unwrap();
        let rec = sample_record();
        store.insert_transaction(&rec).unwrap();
        store
            .update_transaction_state(
                &rec.transaction_id,
                TransactionState::Success,
                Some("0xdeadbeef"),
                None,
            )
            .unwrap();
        let got = store.get_transaction(&rec.transaction_id).unwrap().unwrap();
        assert_eq!(got.state, TransactionState::Success);
        assert_eq!(got.transaction_hash.as_deref(), Some("0xdeadbeef"));
        assert!(got.confirmed_at.is_some());
    }

    #[test]
    fn list_non_terminal_transactions_filters_by_state() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut a = sample_record();
        a.transaction_id = "txn_a".into();
        let mut b = sample_record();
        b.transaction_id = "txn_b".into();
        b.state = TransactionState::Success;
        store.insert_transaction(&a).unwrap();
        store.insert_transaction(&b).unwrap();
        let pending = store.list_non_terminal_transactions().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].transaction_id, "txn_a");
    }

    #[test]
    fn latest_transactions_orders_by_submitted_at_desc_and_limits() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut oldest = sample_record();
        oldest.transaction_id = "txn_oldest".into();
        oldest.submitted_at = chrono::DateTime::parse_from_rfc3339("2026-04-24T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut newest = sample_record();
        newest.transaction_id = "txn_newest".into();
        newest.submitted_at = chrono::DateTime::parse_from_rfc3339("2026-04-25T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);

        store.insert_transaction(&oldest).unwrap();
        store.insert_transaction(&newest).unwrap();

        let latest = store.latest_transactions(1).unwrap();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].transaction_id, "txn_newest");
    }
}

#[cfg(test)]
mod migration_roundtrip_tests {
    use super::SqliteStore;
    use std::path::PathBuf;

    fn tmp_db_path() -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("polybot-phase1-{}.db", uuid::Uuid::new_v4()));
        p
    }

    #[test]
    fn fresh_db_applies_all_migrations_then_reopen_is_noop() {
        let path = tmp_db_path();
        let _ = std::fs::remove_file(&path);

        // First open: apply every migration.
        {
            let store = SqliteStore::open(&path).expect("open fresh db");
            store
                .conn
                .execute_batch(
                    "INSERT INTO transactions
                        (transaction_id, trade_id, type, state, submitted_at)
                     VALUES ('tx1', NULL, 'wrap', 'STATE_NEW', '2026-04-24T00:00:00Z');
                     INSERT INTO signals
                        (id, source, received_at, target_wallet, market_id,
                         token_id, side, outcome, target_price, target_size,
                         confidence, secret_level, category)
                     VALUES ('s1','polling','2026-04-24T00:00:00Z','0xabc','m1',
                             'tok1','YES','YES','0.5','10',7,6,'politics');",
                )
                .expect("v2 tables/columns usable");
        }

        // Second open: migrations already recorded, previously-written row
        // still present, v3 default for taker_fee_bps applied.
        {
            let store = SqliteStore::open(&path).expect("reopen");
            let n: i64 = store
                .conn
                .query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1, "transactions row lost on reopen");

            let taker_fee_bps: i64 = store
                .conn
                .query_row(
                    "SELECT taker_fee_bps FROM signals WHERE id = 's1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(taker_fee_bps, 0, "migration v3 default should be 0");

            let max_version: i64 = store
                .conn
                .query_row(
                    "SELECT COALESCE(MAX(version), -1) FROM schema_migrations",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(
                max_version >= 5,
                "expected all v1-v5 migrations, got {}",
                max_version
            );
        }

        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod insert_trade_v2_tests {
    use super::SqliteStore;
    use polybot_common::types::{
        Category, OrderType, Side, Trade, TradeDirection, TradeStatus, TransactionState,
    };
    use rust_decimal_macros::dec;

    fn v2_trade() -> Trade {
        Trade {
            id: "t-v2".into(),
            signal_id: "s-v2".into(),
            source_wallet: "0xabc123abc123abc123abc123abc123abc123abc1".into(),
            market_id: "m-v2".into(),
            category: Category::Politics,
            side: Side::Yes,
            direction: TradeDirection::Buy,
            price: dec!(0.50),
            size: dec!(100),
            size_usd: dec!(50),
            filled_size: dec!(100),
            order_type: OrderType::Fok,
            status: TradeStatus::Filled,
            placed_at: chrono::Utc::now(),
            filled_at: Some(chrono::Utc::now()),
            simulated: false,
            transaction_id: Some("txn_zzz".into()),
            transaction_hash: Some("0xabc".into()),
            relayer_state: Some(TransactionState::Success),
            taker_fee_bps: 125,
            fee_paid_usdc: dec!(0.25),
            rebate_usdc: dec!(0.05),
            retry_count: 2,
            error_msg: None,
        }
    }

    #[test]
    fn insert_trade_persists_v2_relayer_columns() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.insert_trade(&v2_trade()).unwrap();

        let (txn_id, txn_hash, relayer_state, taker_fee_bps, fee_paid, rebate, retry_count): (
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
            String,
            String,
            i64,
        ) = store
            .conn
            .query_row(
                "SELECT transaction_id, transaction_hash, relayer_state,
                        taker_fee_bps, fee_paid_usdc, rebate_usdc, retry_count
                 FROM trades WHERE id = 't-v2'",
                [],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                    ))
                },
            )
            .unwrap();

        assert_eq!(txn_id.as_deref(), Some("txn_zzz"));
        assert_eq!(txn_hash.as_deref(), Some("0xabc"));
        assert_eq!(relayer_state.as_deref(), Some("STATE_SUCCESS"));
        assert_eq!(taker_fee_bps, 125);
        assert_eq!(fee_paid, "0.25");
        assert_eq!(rebate, "0.05");
        assert_eq!(retry_count, 2);
    }
}
