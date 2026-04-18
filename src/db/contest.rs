use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::types::{Contest, ContestStatus};

#[derive(Debug, sqlx::FromRow)]
struct ContestRow {
    pub id: String,
    pub title: String,
    pub description: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
}

impl ContestRow {
    fn into_contest(self) -> Contest {
        Contest {
            id: self.id,
            title: self.title,
            description: self.description,
            start_time: self.start_time,
            end_time: self.end_time,
        }
    }
}

pub async fn list_all(pool: &PgPool) -> Result<Vec<Contest>> {
    let rows = sqlx::query_as::<_, ContestRow>(
        "SELECT id, title, description, start_time, end_time
         FROM contests ORDER BY start_time DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.into_contest()).collect())
}

/// コンテストに紐付く problem_id のリストを返す（display_order 順）
#[allow(dead_code)]
pub async fn problem_ids(pool: &PgPool, contest_id: &str) -> Result<Vec<(String, String)>> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT label, problem_id FROM contest_problems
         WHERE contest_id = $1 ORDER BY display_order",
    )
    .bind(contest_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// コンテスト一覧を Upcoming / Ongoing / Past に分けて返す
pub struct ContestLists {
    pub ongoing: Vec<Contest>,
    pub upcoming: Vec<Contest>,
    pub past: Vec<Contest>,
}

pub async fn list_grouped(pool: &PgPool) -> Result<ContestLists> {
    let all = list_all(pool).await?;
    let mut ongoing = Vec::new();
    let mut upcoming = Vec::new();
    let mut past = Vec::new();
    for c in all {
        match c.status() {
            ContestStatus::Ongoing  => ongoing.push(c),
            ContestStatus::Upcoming => upcoming.push(c),
            ContestStatus::Past     => past.push(c),
        }
    }
    // upcoming は開催が近い順に
    upcoming.sort_by_key(|c| c.start_time);
    Ok(ContestLists { ongoing, upcoming, past })
}
