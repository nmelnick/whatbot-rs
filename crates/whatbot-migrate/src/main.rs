//! whatbot-migrate: import data from old whatbot
//!
//! ```
//! whatbot-migrate \
//!     --sqlite /path/to/old/whatbot.db \
//!     --postgres "postgres://whatbot@host/whatbot"
//! ```
//!
//! This currently includes factoid and karma.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Context as _;
use chrono::{DateTime, TimeZone, Utc};
use clap::Parser;
use rusqlite::OptionalExtension;
use sqlx::{Postgres, Row, Transaction};
use tracing::{info, warn};
use whatbot_storage::Store;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Import old whatbot SQLite data into the new Postgres schema."
)]
struct Args {
    /// Path to the legacy SQLite database file.
    #[arg(long)]
    sqlite: PathBuf,

    /// Postgres connection string for the new whatbot storage.
    #[arg(long)]
    postgres: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info,sqlx=warn")
        .init();
    let args = Args::parse();
    run(args).await
}

async fn run(args: Args) -> anyhow::Result<()> {
    info!(path = %args.sqlite.display(), "opening sqlite source");
    let sqlite = rusqlite::Connection::open(&args.sqlite)
        .with_context(|| format!("failed to open sqlite at {}", args.sqlite.display()))?;

    info!("connecting to postgres");
    let store = Store::connect(&args.postgres).await?;
    info!("running pending migrations on target");
    store.migrate().await?;

    let mut tx = store.pool().begin().await?;

    info!("step 1/6: account");
    let handle_to_id = upsert_account(&sqlite, &mut tx)
        .await
        .context("step 1/6 (account) failed")?;
    info!(count = handle_to_id.len(), "account imported");

    info!("step 2/6: factoid");
    let factoid_id_map = migrate_factoid(&sqlite, &mut tx)
        .await
        .context("step 2/6 (factoid) failed")?;
    info!(count = factoid_id_map.len(), "factoid imported");

    info!("step 3/6: factoid facts");
    let facts = migrate_factoid_fact(&sqlite, &mut tx, &factoid_id_map, &handle_to_id)
        .await
        .context("step 3/6 (factoid facts) failed")?;
    info!(count = facts, "facts imported");

    info!("step 4/6: karma events");
    let karma = migrate_karma(&sqlite, &mut tx, &handle_to_id)
        .await
        .context("step 4/6 (karma) failed")?;
    info!(count = karma, "karma events imported");

    info!("step 5/6: ignored factoids");
    let n = migrate_ignored(&sqlite, &mut tx)
        .await
        .context("step 5/6 (factoid_ignore) failed")?;
    info!(count = n, "ignored subjects imported as silent factoid");

    info!("step 6/6: user aliases");
    let linked = migrate_user_aliases(&sqlite, &mut tx)
        .await
        .context("step 6/6 (user_alias) failed")?;
    info!(linked, "accounts linked to persons via user_alias");

    tx.commit().await?;
    info!("migration complete");
    Ok(())
}

fn epoch_to_utc(epoch: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(epoch, 0)
        .single()
        .unwrap_or_else(Utc::now)
}

fn int_to_bool(v: Option<i64>) -> bool {
    v.unwrap_or(0) != 0
}

/// SQLite stores raw bytes in text columns and never validates encoding. The
/// old perl whatbot had varying amounts of utf8 support over the span of many
/// years, so it just shoved bytes into SQLite. Now that we're using Postgres,
/// we have a database that cares about the data being put into a field.
fn decode_text(bytes: Vec<u8>, column: &str, row_num: usize) -> String {
    match std::str::from_utf8(&bytes) {
        Ok(_) => String::from_utf8(bytes).expect("validated above"),
        Err(_) => {
            let decoded = cp1252_decode(&bytes);
            warn!(
                row = row_num,
                column = column,
                preview = %decoded,
                "invalid utf8 in legacy data; decoded as cp1252"
            );
            decoded
        }
    }
}

/// Nullable text decode
fn decode_text_opt(bytes: Option<Vec<u8>>, column: &str, row_num: usize) -> Option<String> {
    bytes.map(|b| decode_text(b, column, row_num))
}

/// Decode bytes as Windows-1252 (cp1252), stolen
fn cp1252_decode(bytes: &[u8]) -> String {
    const C1: [Option<char>; 32] = [
        Some('\u{20AC}'), // 0x80 €
        None,             // 0x81
        Some('\u{201A}'), // 0x82 ‚
        Some('\u{0192}'), // 0x83 ƒ
        Some('\u{201E}'), // 0x84 „
        Some('\u{2026}'), // 0x85 …
        Some('\u{2020}'), // 0x86 †
        Some('\u{2021}'), // 0x87 ‡
        Some('\u{02C6}'), // 0x88 ˆ
        Some('\u{2030}'), // 0x89 ‰
        Some('\u{0160}'), // 0x8A Š
        Some('\u{2039}'), // 0x8B ‹
        Some('\u{0152}'), // 0x8C Œ
        None,             // 0x8D
        Some('\u{017D}'), // 0x8E Ž
        None,             // 0x8F
        None,             // 0x90
        Some('\u{2018}'), // 0x91 '
        Some('\u{2019}'), // 0x92 '
        Some('\u{201C}'), // 0x93 "
        Some('\u{201D}'), // 0x94 "
        Some('\u{2022}'), // 0x95 •
        Some('\u{2013}'), // 0x96 –
        Some('\u{2014}'), // 0x97 —
        Some('\u{02DC}'), // 0x98 ˜
        Some('\u{2122}'), // 0x99 ™
        Some('\u{0161}'), // 0x9A š
        Some('\u{203A}'), // 0x9B ›
        Some('\u{0153}'), // 0x9C œ
        None,             // 0x9D
        Some('\u{017E}'), // 0x9E ž
        Some('\u{0178}'), // 0x9F Ÿ
    ];

    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => C1[(b - 0x80) as usize].unwrap_or('\u{FFFD}'),
            other => other as char,
        })
        .collect()
}

/// Pull a text column as raw bytes
fn text_bytes(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Vec<u8>> {
    let value = row.get_ref(index)?;
    match value {
        rusqlite::types::ValueRef::Text(b) => Ok(b.to_vec()),
        rusqlite::types::ValueRef::Blob(b) => Ok(b.to_vec()),
        _ => Err(rusqlite::Error::InvalidColumnType(
            index,
            format!("expected text or blob at column {index}"),
            value.data_type(),
        )),
    }
}

/// Nullable bytes
fn text_bytes_opt(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<Vec<u8>>> {
    let value = row.get_ref(index)?;
    match value {
        rusqlite::types::ValueRef::Null => Ok(None),
        rusqlite::types::ValueRef::Text(b) => Ok(Some(b.to_vec())),
        rusqlite::types::ValueRef::Blob(b) => Ok(Some(b.to_vec())),
        _ => Err(rusqlite::Error::InvalidColumnType(
            index,
            format!("expected text or blob at column {index}"),
            value.data_type(),
        )),
    }
}

fn collect_user_handles(
    sqlite: &rusqlite::Connection,
    table: &str,
    out: &mut Vec<String>,
) -> anyhow::Result<()> {
    let sql = format!("SELECT DISTINCT user FROM {table} WHERE user IS NOT NULL");
    let Ok(mut stmt) = sqlite.prepare(&sql) else {
        return Ok(());
    };
    let col = format!("{table}.user");
    let rows = stmt.query_map([], |row| text_bytes(row, 0))?;
    for (i, h) in rows.enumerate() {
        let bytes = h.with_context(|| format!("while reading distinct user #{i} from {table}"))?;
        out.push(decode_text(bytes, &col, i));
    }
    Ok(())
}

async fn upsert_account(
    sqlite: &rusqlite::Connection,
    tx: &mut Transaction<'_, Postgres>,
) -> anyhow::Result<HashMap<String, i64>> {
    let mut handles: Vec<String> = Vec::new();
    collect_user_handles(sqlite, "factoid_description", &mut handles)?;
    collect_user_handles(sqlite, "karma", &mut handles)?;
    handles.sort();
    handles.dedup();

    let mut map = HashMap::with_capacity(handles.len());
    for handle in handles {
        let row = sqlx::query(
            "INSERT INTO account (service, handle, display)
             VALUES ('legacy', $1, $1)
             ON CONFLICT (service, handle) DO UPDATE
                SET last_seen = now()
             RETURNING id",
        )
        .bind(&handle)
        .fetch_one(&mut **tx)
        .await
        .with_context(|| format!("while upserting account handle={handle:?}"))?;
        let id: i64 = row.get("id");
        map.insert(handle, id);
    }
    Ok(map)
}

async fn migrate_factoid(
    sqlite: &rusqlite::Connection,
    tx: &mut Transaction<'_, Postgres>,
) -> anyhow::Result<HashMap<i64, i64>> {
    let mut stmt = sqlite
        .prepare(
            "SELECT factoid_id, subject, is_or, is_plural, silent, created, updated FROM factoid",
        )
        .context("while preparing SELECT on legacy `factoid` table")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                text_bytes(row, 1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })
        .context("while iterating legacy `factoid` rows")?;

    let mut map = HashMap::new();
    for (row_num, row) in rows.enumerate() {
        let (old_id, subject_bytes, is_or, is_plural, silent, created, updated) = row
            .with_context(|| format!("while reading row #{row_num} from legacy `factoid` table"))?;
        let subject = decode_text(subject_bytes, "factoid.subject", row_num);
        let created_at = created.map(epoch_to_utc).unwrap_or_else(Utc::now);
        let updated_at = updated.map(epoch_to_utc).unwrap_or(created_at);
        let new_id: i64 = sqlx::query_scalar(
            "INSERT INTO factoid
                (subject, subject_norm, is_plural, is_or, silent, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (subject_norm) DO UPDATE
                SET updated_at = EXCLUDED.updated_at
             RETURNING id",
        )
        .bind(&subject)
        .bind(&subject.to_lowercase()) // already lowercased in legacy data
        .bind(int_to_bool(is_plural))
        .bind(int_to_bool(is_or))
        .bind(int_to_bool(silent))
        .bind(created_at)
        .bind(updated_at)
        .fetch_one(&mut **tx)
        .await
        .with_context(|| {
            format!("while migrating factoid #{row_num} (legacy id={old_id}, subject={subject:?})")
        })?;
        map.insert(old_id, new_id);
    }
    Ok(map)
}

async fn migrate_factoid_fact(
    sqlite: &rusqlite::Connection,
    tx: &mut Transaction<'_, Postgres>,
    factoid_id_map: &HashMap<i64, i64>,
    handle_to_id: &HashMap<String, i64>,
) -> anyhow::Result<usize> {
    let mut stmt = sqlite
        .prepare("SELECT factoid_id, description, user, updated FROM factoid_description")
        .context("while preparing SELECT on legacy `factoid_description` table")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                text_bytes(row, 1)?,
                text_bytes_opt(row, 2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .context("while iterating legacy `factoid_description` rows")?;

    let mut count = 0usize;
    for (row_num, row) in rows.enumerate() {
        let (old_factoid_id, description_bytes, user_bytes, updated) = row.with_context(|| {
            format!("while reading row #{row_num} from legacy `factoid_description` table")
        })?;
        let description = decode_text(
            description_bytes,
            "factoid_description.description",
            row_num,
        );
        let user = decode_text_opt(user_bytes, "factoid_description.user", row_num);
        let Some(&new_factoid_id) = factoid_id_map.get(&old_factoid_id) else {
            warn!(
                old_factoid_id,
                row_num, "fact references unknown factoid; skipping"
            );
            continue;
        };
        let account_id = user.as_ref().and_then(|u| handle_to_id.get(u)).copied();
        let created_at = updated.map(epoch_to_utc).unwrap_or_else(Utc::now);
        sqlx::query(
            "INSERT INTO factoid_fact (factoid_id, description, account_id, created_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(new_factoid_id)
        .bind(&description)
        .bind(account_id)
        .bind(created_at)
        .execute(&mut **tx)
        .await
        .with_context(|| {
            format!(
                "while migrating fact #{row_num} \
                 (legacy factoid_id={old_factoid_id}, user={user:?}, description={description:?})"
            )
        })?;
        count += 1;
    }
    Ok(count)
}

async fn migrate_karma(
    sqlite: &rusqlite::Connection,
    tx: &mut Transaction<'_, Postgres>,
    handle_to_id: &HashMap<String, i64>,
) -> anyhow::Result<usize> {
    let mut stmt = sqlite
        .prepare("SELECT subject, user, amount, created FROM karma ORDER BY karma_id")
        .context("while preparing SELECT on legacy `karma` table")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                text_bytes(row, 0)?,
                text_bytes_opt(row, 1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .context("while iterating legacy `karma` rows")?;

    let mut count = 0usize;
    for (row_num, row) in rows.enumerate() {
        let (subject_bytes, user_bytes, amount, created) =
            row.with_context(|| format!("while reading row #{row_num} from legacy `karma` table"))?;
        let subject = decode_text(subject_bytes, "karma.subject", row_num);
        let user = decode_text_opt(user_bytes, "karma.user", row_num);
        let delta: i32 = amount.try_into().unwrap_or(0);
        let account_id = user.as_ref().and_then(|u| handle_to_id.get(u)).copied();
        let created_at = created.map(epoch_to_utc).unwrap_or_else(Utc::now);
        sqlx::query(
            "INSERT INTO karma (subject, subject_norm, delta, account_id, created_at)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&subject)
        .bind(subject.to_lowercase())
        .bind(delta)
        .bind(account_id)
        .bind(created_at)
        .execute(&mut **tx)
        .await
        .with_context(|| {
            format!(
                "while migrating karma row #{row_num} \
                 (subject={subject:?}, delta={delta}, user={user:?})"
            )
        })?;
        count += 1;
    }
    Ok(count)
}

/// Import user_alias rows into person + account.person_id
async fn migrate_user_aliases(
    sqlite: &rusqlite::Connection,
    tx: &mut Transaction<'_, Postgres>,
) -> anyhow::Result<usize> {
    let mut stmt = match sqlite
        .prepare("SELECT user, alias FROM user_alias")
        .optional()?
    {
        Some(s) => s,
        None => {
            info!("user_alias table not found in source; skipping");
            return Ok(0);
        }
    };

    // Build a map: canonical_user → Vec<alias>
    let rows = stmt
        .query_map([], |row| Ok((text_bytes(row, 0)?, text_bytes(row, 1)?)))
        .context("while iterating legacy `user_alias` rows")?;

    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for (i, row) in rows.enumerate() {
        let (user_bytes, alias_bytes) =
            row.with_context(|| format!("while reading user_alias row #{i}"))?;
        let user = decode_text(user_bytes, "user_alias.user", i).to_lowercase();
        let alias = decode_text(alias_bytes, "user_alias.alias", i).to_lowercase();
        groups.entry(user).or_default().push(alias);
    }

    let mut linked = 0usize;
    for (canonical, aliases) in &groups {
        let person_id: i64 = sqlx::query_scalar(
            "INSERT INTO person (display) VALUES ($1)
             ON CONFLICT DO NOTHING
             RETURNING id",
        )
        .bind(canonical)
        .fetch_optional(&mut **tx)
        .await
        .with_context(|| format!("while inserting person for {canonical:?}"))?
        .unwrap_or_else(|| 0i64);

        let person_id = if person_id == 0 {
            sqlx::query_scalar("SELECT id FROM person WHERE LOWER(display) = LOWER($1)")
                .bind(canonical)
                .fetch_one(&mut **tx)
                .await
                .with_context(|| format!("while fetching existing person for {canonical:?}"))?
        } else {
            person_id
        };

        let mut names = aliases.clone();
        names.push(canonical.clone());
        for name in &names {
            let updated = sqlx::query_scalar::<_, i64>(
                "WITH updated AS (
                     UPDATE account SET person_id = $1
                     WHERE LOWER(handle) = $2 AND (person_id IS NULL OR person_id != $1)
                     RETURNING 1
                 )
                 SELECT COUNT(*) FROM updated",
            )
            .bind(person_id)
            .bind(name)
            .fetch_one(&mut **tx)
            .await
            .with_context(|| format!("while linking account {name:?} to person {person_id}"))?;
            linked += updated as usize;
        }
    }
    Ok(linked)
}

async fn migrate_ignored(
    sqlite: &rusqlite::Connection,
    tx: &mut Transaction<'_, Postgres>,
) -> anyhow::Result<usize> {
    let Some(mut stmt) = sqlite
        .prepare("SELECT subject FROM factoid_ignore")
        .optional()
        .context("while preparing SELECT on legacy `factoid_ignore` table")?
    else {
        return Ok(0);
    };
    let rows = stmt
        .query_map([], |row| text_bytes(row, 0))
        .context("while iterating legacy `factoid_ignore` rows")?;
    let mut count = 0usize;
    for (row_num, row) in rows.enumerate() {
        let subject_bytes = row.with_context(|| {
            format!("while reading row #{row_num} from legacy `factoid_ignore` table")
        })?;
        let subject = decode_text(subject_bytes, "factoid_ignore.subject", row_num);
        sqlx::query(
            "INSERT INTO factoid (subject, subject_norm, silent)
             VALUES ($1, $2, true)
             ON CONFLICT (subject_norm) DO UPDATE SET silent = true",
        )
        .bind(&subject)
        .bind(subject.to_lowercase())
        .execute(&mut **tx)
        .await
        .with_context(|| {
            format!("while migrating ignored subject #{row_num} (subject={subject:?})")
        })?;
        count += 1;
    }
    Ok(count)
}
