use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::types::{JudgeStatus, Language, Submission};

/// 単一提出取得用（user_id のみ保持）
#[derive(Debug, sqlx::FromRow)]
struct SubmissionRow {
    id: Uuid,
    user_id: Option<Uuid>,
    problem_id: String,
    language: String,
    source_code: String,
    status: String,
    time_used_ms: Option<i64>,
    memory_used_kb: Option<i64>,
    stdout: Option<String>,
    stderr: Option<String>,
    testcase_results: Option<String>,
    #[allow(dead_code)]
    created_at: DateTime<Utc>,
}

impl SubmissionRow {
    fn into_submission(self) -> Submission {
        let status = JudgeStatus::from_db(&self.status);
        let testcase_results = self
            .testcase_results
            .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok());
        Submission {
            id: self.id,
            user_id: self.user_id,
            problem_id: self.problem_id,
            language: Language::from_db(&self.language),
            source_code: self.source_code,
            status,
            time_used_ms: self.time_used_ms.map(|v| v as u64),
            memory_used_kb: self.memory_used_kb.map(|v| v as u64),
            stdout: self.stdout,
            stderr: self.stderr,
            testcase_results,
        }
    }
}

/// 提出一覧取得用（users テーブル LEFT JOIN でユーザ名付き）
#[derive(Debug, sqlx::FromRow)]
pub struct SubmissionListRow {
    pub id: Uuid,
    #[allow(dead_code)]
    pub user_id: Option<Uuid>,
    pub username: Option<String>,
    pub problem_id: String,
    pub language: String,
    pub status: String,
    pub time_used_ms: Option<i64>,
    pub memory_used_kb: Option<i64>,
    pub testcase_results: Option<String>,
    #[allow(dead_code)]
    pub created_at: DateTime<Utc>,
}

pub async fn insert(pool: &PgPool, sub: &Submission) -> Result<()> {
    sqlx::query(
        "INSERT INTO submissions (id, problem_id, user_id, language, source_code, status)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(sub.id)
    .bind(&sub.problem_id)
    .bind(sub.user_id)
    .bind(sub.language.to_db())
    .bind(&sub.source_code)
    .bind(sub.status.to_db())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_by_id(pool: &PgPool, id: Uuid) -> Result<Option<Submission>> {
    let row = sqlx::query_as::<_, SubmissionRow>(
        "SELECT id, user_id, problem_id, language, source_code, status,
                time_used_ms, memory_used_kb, stdout, stderr, testcase_results, created_at
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
    testcase_results: Option<&[String]>,
) -> Result<()> {
    let tc_json = testcase_results.map(|v| serde_json::to_string(v).unwrap_or_default());
    sqlx::query(
        "UPDATE submissions
         SET status = $1, time_used_ms = $2, memory_used_kb = $3,
             stdout = $4, stderr = $5, testcase_results = $6
         WHERE id = $7",
    )
    .bind(status.to_db())
    .bind(time_used_ms.map(|v| v as i64))
    .bind(memory_used_kb.map(|v| v as i64))
    .bind(stdout)
    .bind(stderr)
    .bind(tc_json)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_recent(pool: &PgPool, limit: i64) -> Result<Vec<SubmissionListRow>> {
    let rows = sqlx::query_as::<_, SubmissionListRow>(
        "SELECT s.id, s.user_id, u.username, s.problem_id, s.language, s.status,
                s.time_used_ms, s.memory_used_kb, s.testcase_results, s.created_at
         FROM submissions s
         LEFT JOIN users u ON s.user_id = u.id
         ORDER BY s.created_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn update_status(pool: &PgPool, id: Uuid, status: &JudgeStatus) -> Result<()> {
    sqlx::query("UPDATE submissions SET status = $1 WHERE id = $2")
        .bind(status.to_db())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}
