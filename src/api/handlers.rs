use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::submission as db_sub;
use crate::types::{JudgeStatus, Submission, SubmitRequest};
use crate::worker::{create_submission, JudgeJob};

use super::AppState;

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// POST /submit
pub async fn submit(
    State(state): State<AppState>,
    Json(req): Json<SubmitRequest>,
) -> Result<Json<Value>, StatusCode> {
    let id = Uuid::new_v4();

    let sub = Submission {
        id,
        source_code: req.source_code.clone(),
        language: req.language.clone(),
        problem_id: req.problem_id.clone(),
        status: JudgeStatus::Pending,
        time_used_ms: None,
        memory_used_kb: None,
        stdout: None,
        stderr: None,
    };

    create_submission(&state.pool, &sub)
        .await
        .map_err(|e| {
            tracing::error!("failed to insert submission: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    state
        .job_tx
        .send(JudgeJob {
            id,
            source_code: req.source_code,
            language: req.language,
            stdin: req.stdin,
            expected_output: req.expected_output,
            time_limit_ms: req.time_limit_ms,
            memory_limit_kb: req.memory_limit_kb,
        })
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    Ok(Json(json!({ "id": id })))
}

/// GET /result/{id}
pub async fn get_result(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Submission>, StatusCode> {
    db_sub::get_by_id(&state.pool, id)
        .await
        .map_err(|e| {
            tracing::error!("db error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}
