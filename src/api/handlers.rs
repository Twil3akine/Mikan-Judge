use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tera::Context;
use uuid::Uuid;

use crate::db::submission as db_sub;
use crate::problem;
use crate::types::{JudgeStatus, Language, Submission, SubmitRequest};
use crate::worker::{create_submission, JudgeJob};

use super::AppState;

// ---- エラー型 ----

pub struct HtmlError(anyhow::Error);

impl IntoResponse for HtmlError {
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Error: {}", self.0)).into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for HtmlError {
    fn from(e: E) -> Self { HtmlError(e.into()) }
}

// ---- ヘルパー ----

fn render(tera: &tera::Tera, template: &str, ctx: Context) -> Result<Html<String>, HtmlError> {
    Ok(Html(tera.render(template, &ctx)?))
}

fn verdict_info(status: &JudgeStatus) -> (&'static str, &'static str, bool) {
    // (verdict label, badge class, is_pending)
    match status {
        JudgeStatus::Pending => ("待機中", "pending", true),
        JudgeStatus::Running => ("ジャッジ中...", "running", true),
        JudgeStatus::Accepted => ("AC", "ac", false),
        JudgeStatus::WrongAnswer => ("WA", "wa", false),
        JudgeStatus::TimeLimitExceeded => ("TLE", "tle", false),
        JudgeStatus::MemoryLimitExceeded => ("MLE", "mle", false),
        JudgeStatus::RuntimeError { .. } => ("RE", "re", false),
        JudgeStatus::CompileError { .. } => ("CE", "ce", false),
        JudgeStatus::InternalError { .. } => ("IE", "ce", false),
    }
}

// ---- HTML ハンドラ ----

pub async fn index() -> Redirect {
    Redirect::to("/problems")
}

pub async fn problems_index(
    State(state): State<AppState>,
) -> Result<Html<String>, HtmlError> {
    let problems = problem::load_all(&state.problems_dir);
    let mut ctx = Context::new();
    ctx.insert("problems", &problems);
    render(&state.tera, "problems/index.html", ctx)
}

pub async fn problems_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Html<String>, HtmlError> {
    let prob = problem::load_one(&state.problems_dir, &id)
        .map_err(|_| HtmlError(anyhow::anyhow!("problem '{id}' not found")))?;
    let mut ctx = Context::new();
    ctx.insert("problem", &prob);
    render(&state.tera, "problems/detail.html", ctx)
}

#[derive(Deserialize)]
pub struct ProblemSubmitForm {
    pub language: String,
    pub source_code: String,
}

pub async fn problems_submit(
    State(state): State<AppState>,
    Path(problem_id): Path<String>,
    Form(form): Form<ProblemSubmitForm>,
) -> Result<Response, HtmlError> {
    let prob = problem::load_one(&state.problems_dir, &problem_id)
        .map_err(|_| HtmlError(anyhow::anyhow!("problem '{problem_id}' not found")))?;

    let language = match form.language.as_str() {
        "rust" => Language::Rust,
        "python" => Language::Python,
        "pypy" => Language::PyPy,
        _ => Language::Cpp,
    };

    // 最初のテストケースで判定（将来: 全テストケース）
    let tc = &prob.testcases[0];

    let id = Uuid::new_v4();
    let sub = Submission {
        id,
        source_code: form.source_code.clone(),
        language: language.clone(),
        problem_id: problem_id.clone(),
        status: JudgeStatus::Pending,
        time_used_ms: None,
        memory_used_kb: None,
        stdout: None,
        stderr: None,
    };

    create_submission(&state.pool, &sub).await?;

    state.job_tx.send(JudgeJob {
        id,
        source_code: form.source_code,
        language,
        stdin: tc.input.clone(),
        expected_output: tc.expected.clone(),
        time_limit_ms: prob.time_limit_ms,
        memory_limit_kb: prob.memory_limit_kb,
    }).await.map_err(|e| HtmlError(anyhow::anyhow!("{e}")))?;

    Ok(Redirect::to(&format!("/submissions/{id}")).into_response())
}

#[derive(Serialize)]
struct SubmissionListItem {
    id: String,
    problem_id: String,
    problem_title: String,
    language: String,
    verdict: &'static str,
    badge_class: &'static str,
    time_used_ms: Option<u64>,
}

pub async fn submissions_index(
    State(state): State<AppState>,
) -> Result<Html<String>, HtmlError> {
    let subs = db_sub::list_recent(&state.pool, 50).await?;

    // problem_id → title のマップを作る（リクエストごとにディスクから読む）
    let problems = problem::load_all(&state.problems_dir);
    let title_map: std::collections::HashMap<&str, &str> = problems
        .iter()
        .map(|p| (p.id.as_str(), p.title.as_str()))
        .collect();

    let rows: Vec<SubmissionListItem> = subs
        .iter()
        .map(|s| {
            let (verdict, badge_class, _) = verdict_info(&s.status);
            let problem_title = title_map
                .get(s.problem_id.as_str())
                .copied()
                .unwrap_or(&s.problem_id)
                .to_string();
            SubmissionListItem {
                id: s.id.to_string(),
                problem_id: s.problem_id.clone(),
                problem_title,
                language: s.language.to_db().to_string(),
                verdict,
                badge_class,
                time_used_ms: s.time_used_ms,
            }
        })
        .collect();
    let mut ctx = Context::new();
    ctx.insert("submissions", &rows);
    render(&state.tera, "submissions/index.html", ctx)
}

pub async fn submissions_detail(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, HtmlError> {
    let sub = db_sub::get_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| HtmlError(anyhow::anyhow!("submission not found")))?;

    let (verdict, badge_class, is_pending) = verdict_info(&sub.status);
    let mut ctx = Context::new();
    ctx.insert("id", &sub.id.to_string());
    ctx.insert("problem_id", &sub.problem_id);
    ctx.insert("language", &sub.language.to_db());
    ctx.insert("source_code", &sub.source_code);
    ctx.insert("verdict", verdict);
    ctx.insert("badge_class", badge_class);
    ctx.insert("is_pending", &is_pending);
    ctx.insert("time_used_ms", &sub.time_used_ms);
    ctx.insert("memory_used_kb", &sub.memory_used_kb);
    ctx.insert("stdout", &sub.stdout);
    ctx.insert("stderr", &sub.stderr);

    render(&state.tera, "submissions/detail.html", ctx)
}

pub async fn submissions_poll(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, HtmlError> {
    let sub = db_sub::get_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| HtmlError(anyhow::anyhow!("submission not found")))?;

    let (verdict, badge_class, is_pending) = verdict_info(&sub.status);
    let mut ctx = Context::new();
    ctx.insert("id", &sub.id.to_string());
    ctx.insert("verdict", verdict);
    ctx.insert("badge_class", badge_class);
    ctx.insert("is_pending", &is_pending);

    render(&state.tera, "submissions/poll.html", ctx)
}

// ---- JSON API (後方互換) ----

pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

pub async fn api_submit(
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

    create_submission(&state.pool, &sub).await.map_err(|e| {
        tracing::error!("failed to insert submission: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    state.job_tx.send(JudgeJob {
        id,
        source_code: req.source_code,
        language: req.language,
        stdin: req.stdin,
        expected_output: req.expected_output,
        time_limit_ms: req.time_limit_ms,
        memory_limit_kb: req.memory_limit_kb,
    }).await.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;

    Ok(Json(json!({ "id": id })))
}

pub async fn api_get_result(
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
