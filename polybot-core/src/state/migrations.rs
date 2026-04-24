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
];
