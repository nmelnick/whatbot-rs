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

#[derive(Debug, Clone)]
pub struct KarmaSubjectScore {
    pub subject: String,
    pub score: i64,
}

#[derive(Debug, Clone)]
pub struct KarmaAggregate {
    pub display: String,
    pub total: i64,
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

    /// Top subjects by karma, limited by `limit`
    pub async fn top_subjects(
        &self,
        ascending: bool,
        limit: i64,
    ) -> Result<Vec<KarmaSubjectScore>, StorageError> {
        let order = if ascending { "ASC" } else { "DESC" };
        let sql = format!(
            "SELECT MIN(subject) AS subject, SUM(delta)::bigint AS total \
             FROM karma \
             GROUP BY subject_norm \
             ORDER BY total {order} \
             LIMIT $1"
        );
        let rows = sqlx::query(&sql).bind(limit).fetch_all(self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| KarmaSubjectScore {
                subject: r.get("subject"),
                score: r.get("total"),
            })
            .collect())
    }

    /// Top controversial, within `limit`
    pub async fn controversial_subjects(
        &self,
        limit: i64,
    ) -> Result<Vec<KarmaSubjectScore>, StorageError> {
        let rows = sqlx::query(
            "SELECT MIN(subject) AS subject, \
                    (SUM(ABS(delta)) - ABS(SUM(delta)))::bigint AS score \
             FROM karma GROUP BY subject_norm \
             ORDER BY score DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| KarmaSubjectScore {
                subject: r.get("subject"),
                score: r.get("score"),
            })
            .collect())
    }

    /// Who scored `subject` in the given direction, ranked by vote count.
    pub async fn scores_for_subject(
        &self,
        subject: &str,
        positive: bool,
    ) -> Result<Vec<KarmaAggregate>, StorageError> {
        let norm = subject.to_lowercase();
        let delta: i32 = if positive { 1 } else { -1 };
        let rows = sqlx::query(
            "SELECT a.display, SUM(k.delta)::bigint AS total \
             FROM karma k \
             JOIN account a ON k.account_id = a.id \
             WHERE k.subject_norm = $1 AND k.delta = $2 \
             GROUP BY a.id, a.display \
             ORDER BY ABS(SUM(k.delta)) DESC",
        )
        .bind(&norm)
        .bind(delta)
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| KarmaAggregate {
                display: r.get("display"),
                total: r.get("total"),
            })
            .collect())
    }

    /// A random subject that `display_name` voted on exclusively
    pub async fn random_exclusive(
        &self,
        display_name: &str,
        positive: bool,
    ) -> Result<Option<String>, StorageError> {
        let name = display_name.to_lowercase();
        let delta: i32 = if positive { 1 } else { -1 };
        let row = sqlx::query(
            "SELECT MIN(k.subject) AS subject \
             FROM karma k \
             JOIN account a ON k.account_id = a.id \
             WHERE k.delta = $1 \
               AND LOWER(a.display) = $2 \
               AND k.subject_norm NOT IN ( \
                   SELECT k2.subject_norm FROM karma k2 \
                   JOIN account a2 ON k2.account_id = a2.id \
                   WHERE k2.delta = $1 AND LOWER(a2.display) != $2 \
               ) \
             GROUP BY k.subject_norm \
             ORDER BY RANDOM() LIMIT 1",
        )
        .bind(delta)
        .bind(&name)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.and_then(|r| r.get("subject")))
    }
}
