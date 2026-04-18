use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tera::Context;
use tower_sessions::Session;
use uuid::Uuid;

use crate::db::{contest as db_contest, submission as db_sub, user as db_user};
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

/// セッションからログイン中のユーザ名を取得する。
async fn current_username(session: &Session, pool: &sqlx::PgPool) -> Option<String> {
    let user_id: Uuid = session.get("user_id").await.ok().flatten()?;
    db_user::find_by_id(pool, user_id).await.ok().flatten().map(|u| u.username)
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    use argon2::{Argon2, PasswordHasher, password_hash::{rand_core::OsRng, SaltString}};
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("hash error: {e}"))?;
    Ok(hash.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::{Argon2, PasswordVerifier, password_hash::PasswordHash};
    let Ok(parsed) = PasswordHash::new(hash) else { return false };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

// ---- 認証ハンドラ ----

pub async fn register_form(
    State(state): State<AppState>,
    session: Session,
) -> Result<Html<String>, HtmlError> {
    let mut ctx = Context::new();
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    ctx.insert("error", &Option::<String>::None);
    render(&state.tera, "auth/register.html", ctx)
}

#[derive(Deserialize)]
pub struct AuthForm {
    pub username: String,
    pub password: String,
}

pub async fn register(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<AuthForm>,
) -> Result<Response, HtmlError> {
    let username = form.username.trim().to_string();
    let password = form.password.trim().to_string();

    // バリデーション
    if username.len() < 3 || username.len() > 20 {
        let mut ctx = Context::new();
        ctx.insert("current_user", &Option::<String>::None);
        ctx.insert("error", &Some("ユーザ名は3〜20文字にしてください"));
        return Ok(render(&state.tera, "auth/register.html", ctx)?.into_response());
    }
    if password.len() < 6 {
        let mut ctx = Context::new();
        ctx.insert("current_user", &Option::<String>::None);
        ctx.insert("error", &Some("パスワードは6文字以上にしてください"));
        return Ok(render(&state.tera, "auth/register.html", ctx)?.into_response());
    }

    // 重複チェック
    if db_user::find_by_username(&state.pool, &username).await?.is_some() {
        let mut ctx = Context::new();
        ctx.insert("current_user", &Option::<String>::None);
        ctx.insert("error", &Some("そのユーザ名はすでに使われています"));
        return Ok(render(&state.tera, "auth/register.html", ctx)?.into_response());
    }

    let hash = hash_password(&password)?;
    let user = db_user::insert(&state.pool, &username, &hash).await?;
    session.insert("user_id", user.id).await
        .map_err(|e| HtmlError(anyhow::anyhow!("session error: {e}")))?;

    Ok(Redirect::to("/problems").into_response())
}

pub async fn login_form(
    State(state): State<AppState>,
    session: Session,
) -> Result<Html<String>, HtmlError> {
    let mut ctx = Context::new();
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    ctx.insert("error", &Option::<String>::None);
    render(&state.tera, "auth/login.html", ctx)
}

pub async fn login(
    State(state): State<AppState>,
    session: Session,
    Form(form): Form<AuthForm>,
) -> Result<Response, HtmlError> {
    let fail = || async {
        let mut ctx = Context::new();
        ctx.insert("current_user", &Option::<String>::None);
        ctx.insert("error", &Some("ユーザ名またはパスワードが違います"));
        render(&state.tera, "auth/login.html", ctx)
    };

    let Some(user) = db_user::find_by_username(&state.pool, form.username.trim()).await? else {
        return Ok(fail().await?.into_response());
    };
    if !verify_password(form.password.trim(), &user.password_hash) {
        return Ok(fail().await?.into_response());
    }

    session.insert("user_id", user.id).await
        .map_err(|e| HtmlError(anyhow::anyhow!("session error: {e}")))?;
    Ok(Redirect::to("/problems").into_response())
}

pub async fn logout(session: Session) -> Result<Response, HtmlError> {
    session.flush().await
        .map_err(|e| HtmlError(anyhow::anyhow!("session error: {e}")))?;
    Ok(Redirect::to("/login").into_response())
}

// ---- HTML ハンドラ ----

pub async fn index(
    State(state): State<AppState>,
    session: Session,
) -> Result<Html<String>, HtmlError> {
    let lists = db_contest::list_grouped(&state.pool).await?;

    #[derive(Serialize)]
    struct ContestItem {
        id: String,
        title: String,
        description: String,
        start_time: String,
        end_time: String,
        status_label: &'static str,
        status_class: &'static str,
    }

    fn to_item(c: &crate::types::Contest) -> ContestItem {
        let st = c.status();
        ContestItem {
            id: c.id.clone(),
            title: c.title.clone(),
            description: c.description.clone(),
            start_time: c.start_time.format("%Y/%m/%d %H:%M").to_string(),
            end_time: c.end_time.format("%Y/%m/%d %H:%M").to_string(),
            status_label: st.label(),
            status_class: st.badge_class(),
        }
    }

    let mut ctx = Context::new();
    ctx.insert("ongoing",  &lists.ongoing.iter().map(to_item).collect::<Vec<_>>());
    ctx.insert("upcoming", &lists.upcoming.iter().map(to_item).collect::<Vec<_>>());
    ctx.insert("past",     &lists.past.iter().map(to_item).collect::<Vec<_>>());
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    render(&state.tera, "index.html", ctx)
}

pub async fn problems_index(
    State(state): State<AppState>,
    session: Session,
) -> Result<Html<String>, HtmlError> {
    let problems = problem::load_all(&state.problems_dir);
    let mut ctx = Context::new();
    ctx.insert("problems", &problems);
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    render(&state.tera, "problems/index.html", ctx)
}

pub async fn problems_detail(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<String>,
) -> Result<Html<String>, HtmlError> {
    let prob = problem::load_one(&state.problems_dir, &id)
        .map_err(|_| HtmlError(anyhow::anyhow!("problem '{id}' not found")))?;
    let mut ctx = Context::new();
    ctx.insert("problem", &prob);
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    render(&state.tera, "problems/detail.html", ctx)
}

#[derive(Deserialize)]
pub struct ProblemSubmitForm {
    pub language: String,
    pub source_code: String,
}

pub async fn problems_submit(
    State(state): State<AppState>,
    session: Session,
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

    // ログイン中なら user_id を紐付ける
    let user_id: Option<Uuid> = session.get("user_id").await.ok().flatten();

    let id = Uuid::new_v4();
    let sub = Submission {
        id,
        user_id,
        source_code: form.source_code.clone(),
        language: language.clone(),
        problem_id: problem_id.clone(),
        status: JudgeStatus::Pending,
        time_used_ms: None,
        memory_used_kb: None,
        stdout: None,
        stderr: None,
        testcase_results: None,
    };

    create_submission(&state.pool, &sub).await?;

    let testcases = prob.testcases.iter()
        .map(|tc| (tc.input.clone(), tc.expected.clone()))
        .collect();

    state.job_tx.send(JudgeJob {
        id,
        source_code: form.source_code,
        language,
        testcases,
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
    username: Option<String>,
    language: String,
    verdict: &'static str,
    badge_class: &'static str,
    time_used_ms: Option<u64>,
    memory_used_kb: Option<u64>,
    tc_summary: Option<String>,
}

pub async fn submissions_index(
    State(state): State<AppState>,
    session: Session,
) -> Result<Html<String>, HtmlError> {
    let rows = db_sub::list_recent(&state.pool, 50).await?;

    let problems = problem::load_all(&state.problems_dir);
    let title_map: std::collections::HashMap<&str, &str> = problems
        .iter()
        .map(|p| (p.id.as_str(), p.title.as_str()))
        .collect();

    let items: Vec<SubmissionListItem> = rows
        .iter()
        .map(|s| {
            let status = JudgeStatus::from_db(&s.status);
            let (verdict, badge_class, _) = verdict_info(&status);
            let problem_title = title_map
                .get(s.problem_id.as_str())
                .copied()
                .unwrap_or(&s.problem_id)
                .to_string();
            let tc_results: Option<Vec<String>> = s.testcase_results
                .as_deref()
                .and_then(|j| serde_json::from_str(j).ok());
            SubmissionListItem {
                id: s.id.to_string(),
                problem_id: s.problem_id.clone(),
                problem_title,
                username: s.username.clone(),
                language: Language::from_db(&s.language).display_name().to_string(),
                verdict,
                badge_class,
                time_used_ms: s.time_used_ms.map(|v| v as u64),
                memory_used_kb: s.memory_used_kb.map(|v| v as u64),
                tc_summary: tc_results.map(|v| {
                    let ac = v.iter().filter(|s| s.as_str() == "AC").count();
                    format!("{ac}/{}", v.len())
                }),
            }
        })
        .collect();

    let mut ctx = Context::new();
    ctx.insert("submissions", &items);
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    render(&state.tera, "submissions/index.html", ctx)
}

pub async fn submissions_detail(
    State(state): State<AppState>,
    session: Session,
    Path(id): Path<Uuid>,
) -> Result<Html<String>, HtmlError> {
    let sub = db_sub::get_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| HtmlError(anyhow::anyhow!("submission not found")))?;

    let (verdict, badge_class, is_pending) = verdict_info(&sub.status);

    let prob = problem::load_one(&state.problems_dir, &sub.problem_id).ok();
    let problem_title = prob.as_ref().map(|p| p.title.clone()).unwrap_or_else(|| sub.problem_id.clone());
    let problem_score = prob.as_ref().map(|p| p.score);

    let lang_hljs = match sub.language.to_db() {
        "pypy" => "python",
        other => other,
    };

    let mut ctx = Context::new();
    ctx.insert("id", &sub.id.to_string());
    ctx.insert("problem_id", &sub.problem_id);
    ctx.insert("problem_title", &problem_title);
    ctx.insert("language", &sub.language.display_name());
    ctx.insert("lang_hljs", lang_hljs);
    ctx.insert("source_code", &sub.source_code);
    ctx.insert("verdict", verdict);
    ctx.insert("badge_class", badge_class);
    ctx.insert("is_pending", &is_pending);
    ctx.insert("time_used_ms", &sub.time_used_ms);
    ctx.insert("memory_used_kb", &sub.memory_used_kb);
    ctx.insert("stdout", &sub.stdout);
    ctx.insert("stderr", &sub.stderr);
    ctx.insert("testcase_results", &sub.testcase_results);
    ctx.insert("problem_score", &problem_score);
    ctx.insert("is_accepted", &matches!(sub.status, JudgeStatus::Accepted));
    ctx.insert("current_user", &current_username(&session, &state.pool).await);

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
        user_id: None,
        source_code: req.source_code.clone(),
        language: req.language.clone(),
        problem_id: req.problem_id.clone(),
        status: JudgeStatus::Pending,
        time_used_ms: None,
        memory_used_kb: None,
        stdout: None,
        stderr: None,
        testcase_results: None,
    };

    create_submission(&state.pool, &sub).await.map_err(|e| {
        tracing::error!("failed to insert submission: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    state.job_tx.send(JudgeJob {
        id,
        source_code: req.source_code,
        language: req.language,
        testcases: vec![(req.stdin, req.expected_output)],
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
