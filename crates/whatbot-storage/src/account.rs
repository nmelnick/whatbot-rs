use sqlx::{PgPool, Row};

use whatbot_core::capability::{Capability, CapabilitySet};
use whatbot_core::{Account, ChannelId, ServiceId};

use crate::error::StorageError;

pub struct AccountRepo<'a> {
    pool: &'a PgPool,
}

impl<'a> AccountRepo<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Insert on first sighting, update otherwise
    pub async fn upsert(
        &self,
        service: &ServiceId,
        handle: &str,
        display: &str,
    ) -> Result<Account, StorageError> {
        let row = sqlx::query(
            r#"
            INSERT INTO account (service, handle, display)
            VALUES ($1, $2, $3)
            ON CONFLICT (service, handle) DO UPDATE
                SET display = EXCLUDED.display,
                    last_seen = now()
            RETURNING id, service, handle, display, person_id
            "#,
        )
        .bind(service.as_str())
        .bind(handle)
        .bind(display)
        .fetch_one(self.pool)
        .await?;

        let id: i64 = row.try_get("id")?;
        let service: String = row.try_get("service")?;
        let handle: String = row.try_get("handle")?;
        let display: String = row.try_get("display")?;
        let person_id: Option<i64> = row.try_get("person_id")?;

        let cap_rows =
            sqlx::query("SELECT capability FROM account_capability WHERE account_id = $1")
                .bind(id)
                .fetch_all(self.pool)
                .await?;
        let capabilities = CapabilitySet::from_caps(
            cap_rows
                .iter()
                .filter_map(|r| r.try_get::<String, _>("capability").ok())
                .map(|s| parse_capability(&s)),
        );

        Ok(Account {
            id,
            service: ServiceId::new(service),
            handle,
            display,
            person_id,
            capabilities,
        })
    }

    /// Grant a capability on an account
    pub async fn grant(
        &self,
        account_id: i64,
        capability: &Capability,
    ) -> Result<(), StorageError> {
        let s = serialize_capability(capability);
        sqlx::query(
            "INSERT INTO account_capability (account_id, capability) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(account_id)
        .bind(&s)
        .execute(self.pool)
        .await?;
        Ok(())
    }

    /// Remove/revoke a capability from an account
    pub async fn revoke(
        &self,
        account_id: i64,
        capability: &Capability,
    ) -> Result<(), StorageError> {
        let s = serialize_capability(capability);
        sqlx::query("DELETE FROM account_capability WHERE account_id = $1 AND capability = $2")
            .bind(account_id)
            .bind(&s)
            .execute(self.pool)
            .await?;
        Ok(())
    }
}

fn serialize_capability(c: &Capability) -> String {
    match c {
        Capability::Admin => "admin".to_string(),
        Capability::Owner => "owner".to_string(),
        Capability::Mod(ch) => format!("mod:{}", ch.as_str()),
        Capability::Custom(s) => format!("custom:{s}"),
    }
}

fn parse_capability(s: &str) -> Capability {
    if s == "admin" {
        Capability::Admin
    } else if s == "owner" {
        Capability::Owner
    } else if let Some(rest) = s.strip_prefix("mod:") {
        Capability::Mod(ChannelId::new(rest))
    } else if let Some(rest) = s.strip_prefix("custom:") {
        Capability::Custom(rest.to_string())
    } else {
        Capability::Custom(s.to_string())
    }
}
