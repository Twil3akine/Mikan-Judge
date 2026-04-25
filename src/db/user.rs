use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::User;

#[derive(Debug, sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    username: String,
    password_hash: String,
    default_language: Option<String>,
}

impl UserRow {
    fn into_user(self) -> User {
        User {
            id: self.id,
            username: self.username,
            password_hash: self.password_hash,
            default_language: self.default_language,
        }
    }
}

pub async fn insert(pool: &PgPool, username: &str, password_hash: &str) -> Result<User> {
    let row = sqlx::query_as::<_, UserRow>(
        "INSERT INTO users (username, password_hash)
         VALUES ($1, $2)
         RETURNING id, username, password_hash, default_language",
    )
    .bind(username)
    .bind(password_hash)
    .fetch_one(pool)
    .await?;
    Ok(row.into_user())
}

pub async fn find_by_username(pool: &PgPool, username: &str) -> Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, default_language FROM users WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.into_user()))
}

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, default_language FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.into_user()))
}

pub async fn update_default_language(
    pool: &PgPool,
    id: Uuid,
    language: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE users SET default_language = $1 WHERE id = $2")
        .bind(language)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &PgPool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
