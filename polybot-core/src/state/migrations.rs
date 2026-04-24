//! Versioned SQLite migrations for polybot-core.
//!
//! Each `Migration` has a monotonically increasing version and a batch of SQL
//! statements. The runner records applied versions in `schema_migrations`.
//! Migrations must be idempotent and additive (no destructive changes without
//! explicit approval).

pub struct Migration {
    pub version: i64,
    pub description: &'static str,
    pub sql: &'static str,
}

/// Ordered list of migrations. Append new entries; never reorder or delete.
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "Baseline V1 tables (idempotent create-if-not-exists)",
        // Baseline is handled by the existing create_tables() block; this
        // entry exists purely to seed schema_migrations so future versions
        // run in order.
        sql: "SELECT 1;",
    },
    Migration {
        version: 2,
        description: "V2: add transactions table for async relayer tracking",
        sql: r#"
            CREATE TABLE IF NOT EXISTS transactions (
                transaction_id   TEXT PRIMARY KEY,
                trade_id         TEXT REFERENCES trades(id),
                type             TEXT NOT NULL,
                state            TEXT NOT NULL,
                submitted_at     TEXT NOT NULL,
                confirmed_at     TEXT,
                transaction_hash TEXT,
                error_msg        TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_transactions_state
                ON transactions(state);
            CREATE INDEX IF NOT EXISTS idx_transactions_trade_id
                ON transactions(trade_id);
        "#,
    },
    Migration {
        version: 3,
        description: "V2: add feeSchedule columns to signals",
        sql: r#"
            ALTER TABLE signals ADD COLUMN taker_fee_bps INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE signals ADD COLUMN maker_fee_bps INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE signals ADD COLUMN rebate_bps    INTEGER NOT NULL DEFAULT 0;
        "#,
    },
    Migration {
        version: 4,
        description: "V2: add relayer + fee columns to trades",
        sql: r#"
            ALTER TABLE trades ADD COLUMN transaction_id   TEXT;
            ALTER TABLE trades ADD COLUMN transaction_hash TEXT;
            ALTER TABLE trades ADD COLUMN relayer_state    TEXT;
            ALTER TABLE trades ADD COLUMN taker_fee_bps    INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE trades ADD COLUMN fee_paid_usdc    TEXT NOT NULL DEFAULT '0';
            ALTER TABLE trades ADD COLUMN rebate_usdc      TEXT NOT NULL DEFAULT '0';
            ALTER TABLE trades ADD COLUMN retry_count      INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE trades ADD COLUMN error_msg        TEXT;
        "#,
    },
];
