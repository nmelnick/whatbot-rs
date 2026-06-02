use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;
use whatbot_core::Account;

#[derive(Debug, Error)]
pub enum FactoidStoreError {
    #[error("backend error: {0}")]
    Backend(String),
}

#[derive(Debug, Clone)]
pub struct Factoid {
    pub id: i64,
    pub subject: String,
    pub is_plural: bool,
    pub is_or: bool,
    pub silent: bool,
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

#[async_trait]
pub trait FactoidStore: Send + Sync {
    async fn find(&self, subject: &str) -> Result<Option<Factoid>, FactoidStoreError>;
    async fn ensure(&self, subject: &str, is_plural: bool) -> Result<i64, FactoidStoreError>;
    async fn add_fact(
        &self,
        factoid_id: i64,
        description: &str,
        account: Option<&Account>,
    ) -> Result<(), FactoidStoreError>;
    async fn facts(&self, factoid_id: i64) -> Result<Vec<FactoidFact>, FactoidStoreError>;
    async fn forget(&self, subject: &str) -> Result<bool, FactoidStoreError>;
}
