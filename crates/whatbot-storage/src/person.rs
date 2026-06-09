use sqlx::{PgPool, Row};

use whatbot_core::{Account, ServiceId};

use crate::error::StorageError;

#[derive(Debug, Clone)]
pub struct Person {
    pub id: i64,
    pub display: String,
}

pub struct PersonRepo<'a> {
    pool: &'a PgPool,
}

impl<'a> PersonRepo<'a> {
    pub fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// Find or create a person by display name
    pub async fn find_or_create(&self, display: &str) -> Result<Person, StorageError> {
        let row = sqlx::query(
            "INSERT INTO person (display)
             SELECT $1 WHERE NOT EXISTS (
                 SELECT 1 FROM person WHERE LOWER(display) = LOWER($1)
             )
             ON CONFLICT DO NOTHING
             RETURNING id, display",
        )
        .bind(display)
        .fetch_optional(self.pool)
        .await?;

        if let Some(r) = row {
            return Ok(Person {
                id: r.get("id"),
                display: r.get("display"),
            });
        }

        let r = sqlx::query("SELECT id, display FROM person WHERE LOWER(display) = LOWER($1)")
            .bind(display)
            .fetch_one(self.pool)
            .await?;
        Ok(Person {
            id: r.get("id"),
            display: r.get("display"),
        })
    }

    /// Find a person by display name
    pub async fn find_by_display(&self, display: &str) -> Result<Option<Person>, StorageError> {
        let row = sqlx::query("SELECT id, display FROM person WHERE LOWER(display) = LOWER($1)")
            .bind(display)
            .fetch_optional(self.pool)
            .await?;
        Ok(row.map(|r| Person {
            id: r.get("id"),
            display: r.get("display"),
        }))
    }

    /// Link an account to a person
    pub async fn link_account(&self, account_id: i64, person_id: i64) -> Result<(), StorageError> {
        sqlx::query("UPDATE account SET person_id = $1 WHERE id = $2")
            .bind(person_id)
            .bind(account_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Unlink an account from a person
    pub async fn unlink_account(&self, account_id: i64) -> Result<(), StorageError> {
        sqlx::query("UPDATE account SET person_id = NULL WHERE id = $1")
            .bind(account_id)
            .execute(self.pool)
            .await?;
        Ok(())
    }

    /// Retrieve all accounts linked to a person
    pub async fn accounts_for_person(&self, person_id: i64) -> Result<Vec<Account>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, service, handle, display, person_id
             FROM account WHERE person_id = $1",
        )
        .bind(person_id)
        .fetch_all(self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Account {
                id: r.get("id"),
                service: ServiceId::new(r.get::<String, _>("service")),
                handle: r.get("handle"),
                display: r.get("display"),
                person_id: r.get("person_id"),
                capabilities: whatbot_core::capability::CapabilitySet::new(),
            })
            .collect())
    }

    /// Find an account by handle
    pub async fn account_by_handle(&self, handle: &str) -> Result<Option<Account>, StorageError> {
        let row = sqlx::query(
            "SELECT id, service, handle, display, person_id
             FROM account WHERE LOWER(handle) = LOWER($1) LIMIT 1",
        )
        .bind(handle)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| Account {
            id: r.get("id"),
            service: ServiceId::new(r.get::<String, _>("service")),
            handle: r.get("handle"),
            display: r.get("display"),
            person_id: r.get("person_id"),
            capabilities: whatbot_core::capability::CapabilitySet::new(),
        }))
    }

    /// Resolve a display name to identity token, or None
    pub async fn identity_id_for_display(
        &self,
        display: &str,
    ) -> Result<Option<i64>, StorageError> {
        let row = sqlx::query(
            "SELECT COALESCE(person_id, id) AS identity_id
             FROM account WHERE LOWER(display) = LOWER($1) LIMIT 1",
        )
        .bind(display)
        .fetch_optional(self.pool)
        .await?;
        Ok(row.map(|r| r.get("identity_id")))
    }
}
