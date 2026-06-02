//! Shared test helpers for whatbot crates that need a real Postgres.

use std::sync::Arc;

use sqlx::{Connection, Executor, PgConnection};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ContainerAsync;
use tokio::sync::OnceCell;
use whatbot_storage::Store;

static PG: OnceCell<Pg> = OnceCell::const_new();

pub struct Pg {
    _container: ContainerAsync<Postgres>,
    host: String,
    port: u16,
}

impl Pg {
    /// Test container singleton
    pub async fn shared() -> &'static Pg {
        PG.get_or_init(Self::start).await
    }

    async fn start() -> Pg {
        let container = Postgres::default()
            .start()
            .await
            .expect("start postgres testcontainer");
        let host = container
            .get_host()
            .await
            .expect("postgres host")
            .to_string();
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("postgres port");
        Pg {
            _container: container,
            host,
            port,
        }
    }

    fn admin_url(&self) -> String {
        format!(
            "postgres://postgres:postgres@{}:{}/postgres",
            self.host, self.port
        )
    }

    fn db_url(&self, db: &str) -> String {
        format!(
            "postgres://postgres:postgres@{}:{}/{}",
            self.host, self.port, db
        )
    }

    pub async fn fresh_store(&self) -> Arc<Store> {
        let db = format!("t_{}", uuid::Uuid::new_v4().simple());
        let mut admin = PgConnection::connect(&self.admin_url())
            .await
            .expect("connect admin");
        admin
            .execute(format!("CREATE DATABASE \"{db}\"").as_str())
            .await
            .expect("create database");
        drop(admin);

        let store = Store::connect(&self.db_url(&db))
            .await
            .expect("connect fresh db");
        store.migrate().await.expect("migrate fresh db");
        Arc::new(store)
    }
}
