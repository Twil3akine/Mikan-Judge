use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::User;

#[derive(Debug, sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    username: String,
    password_hash: String,
}

impl UserRow {
    fn into_user(self) -> User {
        User { id: self.id, username: self.username, password_hash: self.password_hash }
    }
}

pub async fn insert(pool: &PgPool, username: &str, password_hash: &str) -> Result<User> {
    let row = sqlx::query_as::<_, UserRow>(
        "INSERT INTO users (username, password_hash)
         VALUES ($1, $2)
         RETURNING id, username, password_hash",
    )
    .bind(username)
    .bind(password_hash)
    .fetch_one(pool)
    .await?;
    Ok(row.into_user())
}

pub async fn find_by_username(pool: &PgPool, username: &str) -> Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash FROM users WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.into_user()))
}

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.into_user()))
}
