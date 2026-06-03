use async_trait::async_trait;
use sqlx::PgPool;

use whatbot_core::dispatcher::{DispatchError, IdentityResolver};
use whatbot_core::{Account, ServiceId};

use crate::account::AccountRepo;
use crate::error::StorageError;
use crate::factoid::FactoidRepo;
use crate::karma::KarmaRepo;
use crate::seen::SeenRepo;

#[derive(Clone)]
pub struct Store {
    pool: PgPool,
}

impl Store {
    pub async fn connect(database_url: &str) -> Result<Self, StorageError> {
        let pool = PgPool::connect(database_url).await?;
        Ok(Self { pool })
    }

    pub async fn migrate(&self) -> Result<(), StorageError> {
        sqlx::migrate!("../../migrations").run(&self.pool).await?;
        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub fn accounts(&self) -> AccountRepo<'_> {
        AccountRepo::new(&self.pool)
    }

    pub fn factoids(&self) -> FactoidRepo<'_> {
        FactoidRepo::new(&self.pool)
    }

    pub fn karma(&self) -> KarmaRepo<'_> {
        KarmaRepo::new(&self.pool)
    }

    pub fn seen(&self) -> SeenRepo<'_> {
        SeenRepo::new(&self.pool)
    }
}

#[async_trait]
impl IdentityResolver for Store {
    async fn resolve(
        &self,
        service: &ServiceId,
        handle: &str,
        display: &str,
    ) -> Result<Account, DispatchError> {
        self.accounts()
            .upsert(service, handle, display)
            .await
            .map_err(|e| DispatchError::Identity(e.to_string()))
    }
}
