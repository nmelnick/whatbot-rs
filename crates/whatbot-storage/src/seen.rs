use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::error::StorageError;

pub struct SeenRepo<'a> {
    pool: &'a PgPool,
}

#[derive(Debug, Clone)]
pub struct SeenRow {
    pub handle: String,
    pub message: String,
    pub seen_at: DateTime<Utc>,
}

impl<'a> SeenRepo<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    pub async fn record(&self, handle: &str, message: &str) -> Result<(), StorageError> {
        let norm = handle.to_lowercase();
        sqlx::query(
            "INSERT INTO seen (handle, handle_norm, message, seen_at)
             VALUES ($1, $2, $3, now())
             ON CONFLICT (handle_norm) DO UPDATE
             SET handle = EXCLUDED.handle, message = EXCLUDED.message, seen_at = now()",
        )
        .bind(handle)
        .bind(&norm)
        .bind(message)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    pub async fn lookup(&self, handle: &str) -> Result<Option<SeenRow>, StorageError> {
        let norm = handle.to_lowercase();
        let row = sqlx::query(
            "SELECT handle, message, seen_at FROM seen WHERE handle_norm = $1",
        )
        .bind(&norm)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| SeenRow {
            handle: r.get("handle"),
            message: r.get("message"),
            seen_at: r.get("seen_at"),
        }))
    }
}
