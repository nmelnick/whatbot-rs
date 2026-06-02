use std::sync::Arc;

use async_trait::async_trait;

use whatbot_core::Account;
use whatbot_storage::Store;

use super::store::{Factoid, FactoidFact, FactoidStore, FactoidStoreError};

pub struct SqlFactoidStore {
    store: Arc<Store>,
}

impl SqlFactoidStore {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

fn map_err<E: std::fmt::Display>(e: E) -> FactoidStoreError {
    FactoidStoreError::Backend(e.to_string())
}

#[async_trait]
impl FactoidStore for SqlFactoidStore {
    async fn find(&self, subject: &str) -> Result<Option<Factoid>, FactoidStoreError> {
        let row = self.store.factoids().find(subject).await.map_err(map_err)?;
        Ok(row.map(|r| Factoid {
            id: r.id,
            subject: r.subject,
            is_plural: r.is_plural,
            is_or: r.is_or,
            silent: r.silent,
            updated_at: r.updated_at,
        }))
    }

    async fn ensure(&self, subject: &str, is_plural: bool) -> Result<i64, FactoidStoreError> {
        self.store
            .factoids()
            .upsert(subject, is_plural)
            .await
            .map_err(map_err)
    }

    async fn add_fact(
        &self,
        factoid_id: i64,
        description: &str,
        account: Option<&Account>,
    ) -> Result<(), FactoidStoreError> {
        // id == 0 is the synthetic-account marker (no DB row yet); treat as
        // anonymous to avoid blowing up on the account_id FK.
        let account_id = account.map(|a| a.id).filter(|id| *id != 0);
        self.store
            .factoids()
            .add_fact(factoid_id, description, account_id)
            .await
            .map_err(map_err)
    }

    async fn facts(&self, factoid_id: i64) -> Result<Vec<FactoidFact>, FactoidStoreError> {
        let rows = self
            .store
            .factoids()
            .facts(factoid_id)
            .await
            .map_err(map_err)?;
        Ok(rows
            .into_iter()
            .map(|r| FactoidFact {
                id: r.id,
                factoid_id: r.factoid_id,
                description: r.description,
                account_handle: r.account_handle,
                created_at: r.created_at,
            })
            .collect())
    }

    async fn forget(&self, subject: &str) -> Result<bool, FactoidStoreError> {
        self.store.factoids().forget(subject).await.map_err(map_err)
    }
}
