use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::types::{JudgeStatus, Submission, SubmitRequest};
use crate::worker::JudgeJob;

use super::AppState;

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// POST /submit
/// ソースコードを受け取り、ジョブキューに積んで submission id を返す。
pub async fn submit(
    State(state): State<AppState>,
    Json(req): Json<SubmitRequest>,
) -> Result<Json<Value>, StatusCode> {
    let id = Uuid::new_v4();

    // まず Pending 状態で登録
    {
        let mut map = state.store.write().await;
        map.insert(
            id,
            Submission {
                id,
                source_code: req.source_code.clone(),
                language: req.language.clone(),
                problem_id: req.problem_id.clone(),
                status: JudgeStatus::Pending,
                time_used_ms: None,
                memory_used_kb: None,
                stdout: None,
                stderr: None,
            },
        );
    }

    // ジョブをキューに送る
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

/// GET /result/:id
/// 提出結果を返す。id が存在しなければ 404。
pub async fn get_result(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Submission>, StatusCode> {
    let map = state.store.read().await;
    map.get(&id).cloned().map(Json).ok_or(StatusCode::NOT_FOUND)
}
