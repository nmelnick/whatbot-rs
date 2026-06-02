//! Factoid persistence

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::error::StorageError;

pub struct FactoidRepo<'a> {
    pool: &'a PgPool,
}

#[derive(Debug, Clone)]
pub struct Factoid {
    pub id: i64,
    pub subject: String,
    pub is_plural: bool,
    pub is_or: bool,
    pub silent: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct FactoidFact {
    pub id: i64,
    pub factoid_id: i64,
    pub description: String,
    pub account_handle: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl<'a> FactoidRepo<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Look up a factoid by subject
    pub async fn find(&self, subject: &str) -> Result<Option<Factoid>, StorageError> {
        let norm = subject.to_lowercase();
        let row = sqlx::query(
            "SELECT id, subject, is_plural, is_or, silent, created_at, updated_at
             FROM factoid WHERE subject_norm = $1",
        )
        .bind(&norm)
        .fetch_optional(self.pool)
        .await?;

        Ok(row.map(|r| Factoid {
            id: r.get("id"),
            subject: r.get("subject"),
            is_plural: r.get("is_plural"),
            is_or: r.get("is_or"),
            silent: r.get("silent"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }))
    }

    /// Get or create a factoid for a subject, returning the id
    pub async fn upsert(&self, subject: &str, is_plural: bool) -> Result<i64, StorageError> {
        let norm = subject.to_lowercase();
        let row = sqlx::query(
            "INSERT INTO factoid (subject, subject_norm, is_plural)
             VALUES ($1, $2, $3)
             ON CONFLICT (subject_norm) DO UPDATE
                 SET updated_at = now()
             RETURNING id",
        )
        .bind(subject)
        .bind(&norm)
        .bind(is_plural)
        .fetch_one(self.pool)
        .await?;
        Ok(row.get("id"))
    }

    /// Add a fact to an existing factoid
    pub async fn add_fact(
        &self,
        factoid_id: i64,
        description: &str,
        account_id: Option<i64>,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO factoid_fact (factoid_id, description, account_id)
             VALUES ($1, $2, $3)",
        )
        .bind(factoid_id)
        .bind(description)
        .bind(account_id)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Get all facts for an existing factoid
    pub async fn facts(&self, factoid_id: i64) -> Result<Vec<FactoidFact>, StorageError> {
        let rows = sqlx::query(
            "SELECT f.id, f.factoid_id, f.description, f.created_at,
                    a.handle AS account_handle
             FROM factoid_fact f
             LEFT JOIN account a ON a.id = f.account_id
             WHERE f.factoid_id = $1
             ORDER BY f.id",
        )
        .bind(factoid_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| FactoidFact {
                id: r.get("id"),
                factoid_id: r.get("factoid_id"),
                description: r.get("description"),
                account_handle: r.try_get("account_handle").ok(),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Delete the factoid and cascade
    pub async fn forget(&self, subject: &str) -> Result<bool, StorageError> {
        let norm = subject.to_lowercase();
        let result = sqlx::query("DELETE FROM factoid WHERE subject_norm = $1")
            .bind(&norm)
            .execute(self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Silence (or not) a subject
    pub async fn toggle_silent(&self, subject: &str) -> Result<Option<bool>, StorageError> {
        let norm = subject.to_lowercase();
        let row = sqlx::query(
            "UPDATE factoids SET silent = NOT silent WHERE subject_norm = $1 RETURNING silent",
        )
        .bind(&norm)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| r.get::<bool, _>("silent")))
    }
}
