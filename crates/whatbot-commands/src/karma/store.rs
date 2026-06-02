use async_trait::async_trait;
use thiserror::Error;
use whatbot_core::Account;

#[derive(Debug, Error)]
pub enum KarmaStoreError {
    #[error("backend error: {0}")]
    Backend(String),
}

#[async_trait]
pub trait KarmaStore: Send + Sync {
    async fn score(&self, subject: &str) -> Result<Option<i32>, KarmaStoreError>;
    async fn apply(
        &self,
        subject: &str,
        delta: i32,
        account: Option<&Account>,
    ) -> Result<i32, KarmaStoreError>;
}
