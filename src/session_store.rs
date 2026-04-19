use async_trait::async_trait;
use sqlx::PgPool;
use time::OffsetDateTime;
use tower_sessions::session::{Id, Record};
use tower_sessions::session_store::{self, SessionStore};

/// PostgreSQL-backed session store for tower-sessions.
/// Sessions are stored in the `tower_sessions` table.
#[derive(Clone, Debug)]
pub struct PgSessionStore {
    pool: PgPool,
}

impl PgSessionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn enc(e: impl std::fmt::Display) -> session_store::Error {
    session_store::Error::Encode(e.to_string())
}
fn dec(e: impl std::fmt::Display) -> session_store::Error {
    session_store::Error::Decode(e.to_string())
}
fn bk(e: impl std::fmt::Display) -> session_store::Error {
    session_store::Error::Backend(e.to_string())
}

#[async_trait]
impl SessionStore for PgSessionStore {
    async fn save(&self, record: &Record) -> session_store::Result<()> {
        let id = record.id.to_string();
        let data = serde_json::to_string(&record.data).map_err(enc)?;
        let expiry = record.expiry_date.unix_timestamp();
        sqlx::query(
            "INSERT INTO tower_sessions (id, data, expiry_unix)
             VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET data = $2, expiry_unix = $3",
        )
        .bind(&id)
        .bind(&data)
        .bind(expiry)
        .execute(&self.pool)
        .await
        .map_err(bk)?;
        Ok(())
    }

    async fn load(&self, session_id: &Id) -> session_store::Result<Option<Record>> {
        let id = session_id.to_string();
        let row: Option<(String, i64)> =
            sqlx::query_as("SELECT data, expiry_unix FROM tower_sessions WHERE id = $1")
                .bind(&id)
                .fetch_optional(&self.pool)
                .await
                .map_err(bk)?;

        let Some((data_str, expiry_unix)) = row else {
            return Ok(None);
        };

        let expiry_date =
            OffsetDateTime::from_unix_timestamp(expiry_unix).map_err(|e| dec(e))?;

        // Expired session: clean up and return None
        if expiry_date < OffsetDateTime::now_utc() {
            let _ = self.delete(session_id).await;
            return Ok(None);
        }

        let data = serde_json::from_str(&data_str).map_err(dec)?;
        Ok(Some(Record {
            id: *session_id,
            data,
            expiry_date,
        }))
    }

    async fn delete(&self, session_id: &Id) -> session_store::Result<()> {
        let id = session_id.to_string();
        sqlx::query("DELETE FROM tower_sessions WHERE id = $1")
            .bind(&id)
            .execute(&self.pool)
            .await
            .map_err(bk)?;
        Ok(())
    }
}
