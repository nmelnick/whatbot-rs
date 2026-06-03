use async_trait::async_trait;
use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct SeenRecord {
    pub handle: String,
    pub message: String,
    pub seen_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum SeenStoreError {
    #[error("backend error: {0}")]
    Backend(String),
}

#[async_trait]
pub trait SeenStore: Send + Sync {
    async fn record(&self, handle: &str, message: &str) -> Result<(), SeenStoreError>;
    async fn lookup(&self, handle: &str) -> Result<Option<SeenRecord>, SeenStoreError>;
}
