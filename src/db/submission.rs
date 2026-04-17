use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::{JudgeStatus, Language, Submission};

/// DB の行に対応する構造体（sqlx の FromRow が使える型のみ）
#[derive(Debug, sqlx::FromRow)]
struct SubmissionRow {
    id: Uuid,
    problem_id: String,
    language: String,
    source_code: String,
    status: String,
    time_used_ms: Option<i64>,
    memory_used_kb: Option<i64>,
    stdout: Option<String>,
    stderr: Option<String>,
    #[allow(dead_code)]
    created_at: DateTime<Utc>,
}

impl SubmissionRow {
    fn into_submission(self) -> Submission {
        let status = JudgeStatus::from_db(&self.status);
        Submission {
            id: self.id,
            problem_id: self.problem_id,
            language: Language::from_db(&self.language),
            source_code: self.source_code,
            status,
            time_used_ms: self.time_used_ms.map(|v| v as u64),
            memory_used_kb: self.memory_used_kb.map(|v| v as u64),
            stdout: self.stdout,
            stderr: self.stderr,
        }
    }
}

pub async fn insert(pool: &PgPool, sub: &Submission) -> Result<()> {
    sqlx::query(
        "INSERT INTO submissions (id, problem_id, language, source_code, status)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(sub.id)
    .bind(&sub.problem_id)
    .bind(sub.language.to_db())
    .bind(&sub.source_code)
    .bind(sub.status.to_db())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Submission>> {
    let row = sqlx::query_as::<_, SubmissionRow>(
        "SELECT id, problem_id, language, source_code, status,
                time_used_ms, memory_used_kb, stdout, stderr, created_at
         FROM submissions WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.into_submission()))
}

pub async fn update_result(
    pool: &PgPool,
    id: Uuid,
    status: &JudgeStatus,
    time_used_ms: Option<u64>,
    memory_used_kb: Option<u64>,
    stdout: Option<&str>,
    stderr: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE submissions
         SET status = $1, time_used_ms = $2, memory_used_kb = $3,
             stdout = $4, stderr = $5
         WHERE id = $6",
    )
    .bind(status.to_db())
    .bind(time_used_ms.map(|v| v as i64))
    .bind(memory_used_kb.map(|v| v as i64))
    .bind(stdout)
    .bind(stderr)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: &JudgeStatus) -> Result<()> {
    sqlx::query("UPDATE submissions SET status = $1 WHERE id = $2")
        .bind(status.to_db())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
