use std::sync::Arc;

use async_trait::async_trait;

use whatbot_core::Account;
use whatbot_storage::Store;

use super::store::{KarmaStore, KarmaStoreError};

pub struct SqlKarmaStore {
    store: Arc<Store>,
}

impl SqlKarmaStore {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl KarmaStore for SqlKarmaStore {
    async fn score(&self, subject: &str) -> Result<Option<i32>, KarmaStoreError> {
        self.store
            .karma()
            .score(subject)
            .await
            .map_err(|e| KarmaStoreError::Backend(e.to_string()))
    }

    async fn apply(
        &self,
        subject: &str,
        delta: i32,
        account: Option<&Account>,
    ) -> Result<i32, KarmaStoreError> {
        self.store
            .karma()
            .record(
                subject,
                delta,
                // Synthetic accounts (id == 0) have no row to FK against.
                account.map(|a| a.id).filter(|id| *id != 0),
            )
            .await
            .map_err(|e| KarmaStoreError::Backend(e.to_string()))
    }
}
