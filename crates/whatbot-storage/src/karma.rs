//! Karma persistence as a per-event log.
//!
//! Each ++/-- is a row. The current score is `SUM(delta)` over rows
//! sharing a `subject_norm`.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::error::StorageError;

pub struct KarmaRepo<'a> {
    pool: &'a PgPool,
}

/// One event row.
#[derive(Debug, Clone)]
pub struct KarmaEvent {
    pub id: i64,
    pub subject: String,
    pub delta: i32,
    pub account_id: Option<i64>,
    pub created_at: DateTime<Utc>,
}

impl<'a> KarmaRepo<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Aggregate score for a subject, or None if the subject is missing
    pub async fn score(&self, subject: &str) -> Result<Option<i32>, StorageError> {
        let norm = subject.to_lowercase();
        // SUM(int) is NULL for no rows; cast to int so sqlx gives back i32.
        let row = sqlx::query("SELECT SUM(delta)::int AS s FROM karma WHERE subject_norm = $1")
            .bind(&norm)
            .fetch_one(self.pool)
            .await?;
        Ok(row.try_get::<Option<i32>, _>("s")?)
    }

    /// Record a karma event and return the new score
    pub async fn record(
        &self,
        subject: &str,
        delta: i32,
        account_id: Option<i64>,
    ) -> Result<i32, StorageError> {
        let norm = subject.to_lowercase();
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO karma (subject, subject_norm, delta, account_id)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(subject)
        .bind(&norm)
        .bind(delta)
        .bind(account_id)
        .execute(&mut *tx)
        .await?;

        let row = sqlx::query("SELECT SUM(delta)::int AS s FROM karma WHERE subject_norm = $1")
            .bind(&norm)
            .fetch_one(&mut *tx)
            .await?;
        let score: i32 = row.try_get::<Option<i32>, _>("s")?.unwrap_or(0);

        tx.commit().await?;
        Ok(score)
    }

    /// Return every event for a subject, oldest first
    pub async fn events(&self, subject: &str) -> Result<Vec<KarmaEvent>, StorageError> {
        let norm = subject.to_lowercase();
        let rows = sqlx::query(
            "SELECT id, subject, delta, account_id, created_at
             FROM karma WHERE subject_norm = $1 ORDER BY id",
        )
        .bind(&norm)
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| KarmaEvent {
                id: r.get("id"),
                subject: r.get("subject"),
                delta: r.get("delta"),
                account_id: r.get("account_id"),
                created_at: r.get("created_at"),
            })
            .collect())
    }
}
