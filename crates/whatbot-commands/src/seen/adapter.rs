use std::sync::Arc;

use async_trait::async_trait;
use whatbot_storage::Store;

use super::store::{SeenRecord, SeenStore, SeenStoreError};

pub struct SqlSeenStore {
    store: Arc<Store>,
}

impl SqlSeenStore {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

fn map_err<E: std::fmt::Display>(e: E) -> SeenStoreError {
    SeenStoreError::Backend(e.to_string())
}

#[async_trait]
impl SeenStore for SqlSeenStore {
    async fn record(&self, handle: &str, message: &str) -> Result<(), SeenStoreError> {
        self.store
            .seen()
            .record(handle, message)
            .await
            .map_err(map_err)
    }

    async fn lookup(&self, handle: &str) -> Result<Option<SeenRecord>, SeenStoreError> {
        let row = self.store.seen().lookup(handle).await.map_err(map_err)?;
        Ok(row.map(|r| SeenRecord {
            handle: r.handle,
            message: r.message,
            seen_at: r.seen_at,
        }))
    }
}
