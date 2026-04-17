use anyhow::Result;
use sqlx::PgPool;

pub mod contest;
pub mod submission;
pub mod user;

pub async fn create_pool(database_url: &str) -> Result<PgPool> {
    let pool = PgPool::connect(database_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    tracing::info!("Database connected and migrations applied");
    Ok(pool)
}
