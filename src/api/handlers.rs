use axum::{
    extract::{Form, Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json, Redirect, Response},
};
use chrono::Utc;
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

/// ページネーションの表示項目を構築する。0 は省略記号（…）を表す。
fn build_pagination(current: i64, total: i64) -> Vec<i64> {
    if total <= 7 {
        return (1..=total).collect();
    }
    let mut items: Vec<i64> = Vec::new();
    let window = 2i64; // current の前後何ページ表示するか
    let left = (current - window).max(2);
    let right = (current + window).min(total - 1);

    items.push(1);
    if left > 2 { items.push(0); } // 省略記号
    for n in left..=right { items.push(n); }
    if right < total - 1 { items.push(0); } // 省略記号
    items.push(total);
    items
}

/// 提出クールダウンをチェックし、残りミリ秒を返す（0 なら OK）
async fn check_submit_cooldown(session: &Session) -> i64 {
    const COOLDOWN_MS: i64 = 5000;
    let last_ms: i64 = session.get("last_submit_at").await.ok().flatten().unwrap_or(0);
    let now_ms = Utc::now().timestamp_millis();
    let elapsed = now_ms - last_ms;
    if elapsed < COOLDOWN_MS { COOLDOWN_MS - elapsed } else { 0 }
}

async fn record_submit_time(session: &Session) {
    let now_ms = Utc::now().timestamp_millis();
    let _ = session.insert("last_submit_at", now_ms).await;
}

// ---- 認証ハンドラ ----

pub async fn register_form(
    State(state): State<AppState>,
    session: Session,
) -> Result<Html<String>, HtmlError> {
    let mut ctx = Context::new();
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    ctx.insert("contest_id", &Option::<String>::None);
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
        ctx.insert("contest_id", &Option::<String>::None);
        ctx.insert("error", &Some("ユーザ名は3〜20文字にしてください"));
        return Ok(render(&state.tera, "auth/register.html", ctx)?.into_response());
    }
    if password.len() < 6 {
        let mut ctx = Context::new();
        ctx.insert("current_user", &Option::<String>::None);
        ctx.insert("contest_id", &Option::<String>::None);
        ctx.insert("error", &Some("パスワードは6文字以上にしてください"));
        return Ok(render(&state.tera, "auth/register.html", ctx)?.into_response());
    }

    // 重複チェック
    if db_user::find_by_username(&state.pool, &username).await?.is_some() {
        let mut ctx = Context::new();
        ctx.insert("current_user", &Option::<String>::None);
        ctx.insert("contest_id", &Option::<String>::None);
        ctx.insert("error", &Some("そのユーザ名はすでに使われています"));
        return Ok(render(&state.tera, "auth/register.html", ctx)?.into_response());
    }

    let hash = hash_password(&password)?;
    let user = db_user::insert(&state.pool, &username, &hash).await?;
    session.insert("user_id", user.id).await
        .map_err(|e| HtmlError(anyhow::anyhow!("session error: {e}")))?;

    Ok(Redirect::to("/").into_response())
}

pub async fn login_form(
    State(state): State<AppState>,
    session: Session,
) -> Result<Html<String>, HtmlError> {
    let mut ctx = Context::new();
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    ctx.insert("contest_id", &Option::<String>::None);
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
        ctx.insert("contest_id", &Option::<String>::None);
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
    Ok(Redirect::to("/").into_response())
}

pub async fn logout(session: Session) -> Result<Response, HtmlError> {
    session.flush().await
        .map_err(|e| HtmlError(anyhow::anyhow!("session error: {e}")))?;
    Ok(Redirect::to("/login").into_response())
}

// ---- トップページ（コンテスト一覧） ----

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
    ctx.insert("contest_id", &Option::<String>::None);
    render(&state.tera, "index.html", ctx)
}

// ---- コンテスト詳細（→ 問題一覧へリダイレクト） ----

pub async fn contest_detail(
    Path(contest_id): Path<String>,
) -> Redirect {
    Redirect::to(&format!("/contests/{}/problems", contest_id))
}

// ---- コンテスト内 問題一覧 ----

pub async fn contest_problems_index(
    State(state): State<AppState>,
    session: Session,
    Path(contest_id): Path<String>,
) -> Result<Html<String>, HtmlError> {
    let contest = db_contest::get_by_id(&state.pool, &contest_id)
        .await?
        .ok_or_else(|| HtmlError(anyhow::anyhow!("contest not found")))?;

    let cp_list = db_contest::problems_for_contest(&state.pool, &contest_id).await?;

    #[derive(Serialize)]
    struct ProblemItem {
        label: String,
        id: String,
        title: String,
        score: u64,
        time_limit_ms: u64,
        memory_limit_kb: u64,
    }

    let mut problems: Vec<ProblemItem> = Vec::new();
    for cp in &cp_list {
        if let Ok(prob) = problem::load_one(&state.problems_dir, &cp.problem_id) {
            problems.push(ProblemItem {
                label: cp.label.clone(),
                id: prob.id.clone(),
                title: prob.title.clone(),
                score: prob.score,
                time_limit_ms: prob.time_limit_ms,
                memory_limit_kb: prob.memory_limit_kb,
            });
        }
    }

    let mut ctx = Context::new();
    ctx.insert("contest_id", &contest_id);
    ctx.insert("contest_title", &contest.title);
    ctx.insert("problems", &problems);
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    render(&state.tera, "contests/problems/index.html", ctx)
}

// ---- コンテスト内 問題詳細 ----

#[derive(Deserialize)]
pub struct ProblemDetailQuery {
    pub cooldown_remaining_ms: Option<i64>,
}

pub async fn contest_problem_detail(
    State(state): State<AppState>,
    session: Session,
    Path((contest_id, problem_id)): Path<(String, String)>,
    Query(query): Query<ProblemDetailQuery>,
) -> Result<Html<String>, HtmlError> {
    let contest = db_contest::get_by_id(&state.pool, &contest_id)
        .await?
        .ok_or_else(|| HtmlError(anyhow::anyhow!("contest not found")))?;

    // コンテストにこの問題が含まれているか確認
    let cp_list = db_contest::problems_for_contest(&state.pool, &contest_id).await?;
    let label = cp_list
        .iter()
        .find(|cp| cp.problem_id == problem_id)
        .map(|cp| cp.label.clone())
        .ok_or_else(|| HtmlError(anyhow::anyhow!("problem not in contest")))?;

    let prob = problem::load_one(&state.problems_dir, &problem_id)
        .map_err(|_| HtmlError(anyhow::anyhow!("problem '{problem_id}' not found")))?;

    let mut ctx = Context::new();
    ctx.insert("contest_id", &contest_id);
    ctx.insert("contest_title", &contest.title);
    ctx.insert("problem", &prob);
    ctx.insert("problem_label", &label);
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    ctx.insert("cooldown_remaining_ms", &query.cooldown_remaining_ms);
    render(&state.tera, "contests/problems/detail.html", ctx)
}

// ---- コンテスト内 提出 ----

#[derive(Deserialize)]
pub struct ProblemSubmitForm {
    pub language: String,
    pub source_code: String,
}

pub async fn contest_problem_submit(
    State(state): State<AppState>,
    session: Session,
    Path((contest_id, problem_id)): Path<(String, String)>,
    Form(form): Form<ProblemSubmitForm>,
) -> Result<Response, HtmlError> {
    // クールダウンチェック
    let remaining = check_submit_cooldown(&session).await;
    if remaining > 0 {
        return Ok(Redirect::to(&format!(
            "/contests/{contest_id}/problems/{problem_id}?cooldown_remaining_ms={remaining}"
        ))
        .into_response());
    }

    let prob = problem::load_one(&state.problems_dir, &problem_id)
        .map_err(|_| HtmlError(anyhow::anyhow!("problem '{problem_id}' not found")))?;

    let language = match form.language.as_str() {
        "rust" => Language::Rust,
        "python" => Language::Python,
        "pypy" => Language::PyPy,
        _ => Language::Cpp,
    };

    let user_id: Option<Uuid> = session.get("user_id").await.ok().flatten();

    let id = Uuid::new_v4();
    let sub = Submission {
        id,
        user_id,
        contest_id: Some(contest_id.clone()),
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
    record_submit_time(&session).await;

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

    Ok(Redirect::to(&format!("/contests/{contest_id}/submissions/{id}")).into_response())
}

// ---- コンテスト内 提出一覧（ページネーション付き） ----

#[derive(Deserialize)]
pub struct PageQuery {
    pub page: Option<i64>,
}

#[derive(Serialize)]
struct SubmissionListItem {
    id: String,
    problem_id: String,
    problem_label: String,
    problem_title: String,
    username: Option<String>,
    language: String,
    verdict: &'static str,
    badge_class: &'static str,
    time_used_ms: Option<u64>,
    memory_used_kb: Option<u64>,
    tc_summary: Option<String>,
}

pub async fn contest_submissions_index(
    State(state): State<AppState>,
    session: Session,
    Path(contest_id): Path<String>,
    Query(pq): Query<PageQuery>,
) -> Result<Html<String>, HtmlError> {
    const PER_PAGE: i64 = 20;
    let page = pq.page.unwrap_or(1).max(1);

    let contest = db_contest::get_by_id(&state.pool, &contest_id)
        .await?
        .ok_or_else(|| HtmlError(anyhow::anyhow!("contest not found")))?;

    let total = db_sub::count_for_contest(&state.pool, &contest_id).await?;
    let total_pages = ((total + PER_PAGE - 1) / PER_PAGE).max(1);
    let page = page.min(total_pages);

    let rows = db_sub::list_for_contest(&state.pool, &contest_id, page, PER_PAGE).await?;

    // problem label map
    let cp_list = db_contest::problems_for_contest(&state.pool, &contest_id).await?;
    let label_map: std::collections::HashMap<&str, &str> =
        cp_list.iter().map(|cp| (cp.problem_id.as_str(), cp.label.as_str())).collect();

    let problems = problem::load_all(&state.problems_dir);
    let title_map: std::collections::HashMap<&str, &str> =
        problems.iter().map(|p| (p.id.as_str(), p.title.as_str())).collect();

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
            let problem_label = label_map
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
                problem_label,
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

    // ページネーション: 表示するページ番号を計算（0 = 省略記号）
    let pagination = build_pagination(page, total_pages);

    let mut ctx = Context::new();
    ctx.insert("contest_id", &contest_id);
    ctx.insert("contest_title", &contest.title);
    ctx.insert("submissions", &items);
    ctx.insert("current_page", &page);
    ctx.insert("total_pages", &total_pages);
    ctx.insert("pagination", &pagination);
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    render(&state.tera, "contests/submissions/index.html", ctx)
}

// ---- コンテスト内 提出詳細 ----

pub async fn contest_submission_detail(
    State(state): State<AppState>,
    session: Session,
    Path((contest_id, id)): Path<(String, Uuid)>,
) -> Result<Html<String>, HtmlError> {
    let contest = db_contest::get_by_id(&state.pool, &contest_id)
        .await?
        .ok_or_else(|| HtmlError(anyhow::anyhow!("contest not found")))?;

    let sub = db_sub::get_by_id(&state.pool, id)
        .await?
        .ok_or_else(|| HtmlError(anyhow::anyhow!("submission not found")))?;

    let (verdict, badge_class, is_pending) = verdict_info(&sub.status);

    let prob = problem::load_one(&state.problems_dir, &sub.problem_id).ok();
    let problem_title = prob.as_ref().map(|p| p.title.clone()).unwrap_or_else(|| sub.problem_id.clone());
    let problem_score = prob.as_ref().map(|p| p.score);

    // problem label
    let cp_list = db_contest::problems_for_contest(&state.pool, &contest_id).await?;
    let problem_label = cp_list
        .iter()
        .find(|cp| cp.problem_id == sub.problem_id)
        .map(|cp| cp.label.clone())
        .unwrap_or_default();

    let lang_hljs = match sub.language.to_db() {
        "pypy" => "python",
        other => other,
    };

    let mut ctx = Context::new();
    ctx.insert("contest_id", &contest_id);
    ctx.insert("contest_title", &contest.title);
    ctx.insert("id", &sub.id.to_string());
    ctx.insert("problem_id", &sub.problem_id);
    ctx.insert("problem_title", &problem_title);
    ctx.insert("problem_label", &problem_label);
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
    render(&state.tera, "contests/submissions/detail.html", ctx)
}

// ---- コンテスト内 提出詳細 (htmx poll) ----

pub async fn contest_submission_poll(
    State(state): State<AppState>,
    Path((_contest_id, id)): Path<(String, Uuid)>,
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

// ---- 順位表 ----

pub async fn contest_standings(
    State(state): State<AppState>,
    session: Session,
    Path(contest_id): Path<String>,
) -> Result<Html<String>, HtmlError> {
    let contest = db_contest::get_by_id(&state.pool, &contest_id)
        .await?
        .ok_or_else(|| HtmlError(anyhow::anyhow!("contest not found")))?;

    let cp_list = db_contest::problems_for_contest(&state.pool, &contest_id).await?;

    // 問題スコアの map
    let mut score_map: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for cp in &cp_list {
        if let Ok(prob) = problem::load_one(&state.problems_dir, &cp.problem_id) {
            score_map.insert(cp.problem_id.clone(), prob.score);
        }
    }

    let first_acs = db_sub::first_acs_for_contest(&state.pool, &contest_id).await?;

    // ユーザごとに集計
    // key: user_id
    use std::collections::HashMap;
    use uuid::Uuid;
    use chrono::DateTime;
    use chrono::Utc;

    struct UserData {
        username: String,
        solved: HashMap<String, DateTime<Utc>>, // problem_id -> first_ac_at
    }

    let mut user_map: HashMap<Uuid, UserData> = HashMap::new();
    for row in &first_acs {
        let entry = user_map.entry(row.user_id).or_insert_with(|| UserData {
            username: row.username.clone(),
            solved: HashMap::new(),
        });
        entry.solved.insert(row.problem_id.clone(), row.first_ac_at);
    }

    #[derive(Serialize)]
    struct ProblemCell {
        label: String,
        ac_time: Option<String>,
        score: u64,
    }

    fn fmt_elapsed(secs: i64) -> String {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        format!("{h}:{m:02}:{s:02}")
    }

    #[derive(Serialize)]
    struct StandingRow {
        rank: usize,
        username: String,
        total_score: u64,
        /// コンテスト開始からの経過時間（表示用）
        elapsed_from_start: Option<String>,
        problems: Vec<ProblemCell>,
    }

    // ソート用に生の DateTime を保持する中間構造体
    struct RowWithRaw {
        row: StandingRow,
        last_ac_raw: Option<DateTime<Utc>>,
    }

    let mut rows_raw: Vec<RowWithRaw> = user_map
        .values()
        .map(|ud| {
            let mut total_score: u64 = 0;
            let mut last_ac: Option<DateTime<Utc>> = None;
            let problems: Vec<ProblemCell> = cp_list
                .iter()
                .map(|cp| {
                    let ac_at = ud.solved.get(&cp.problem_id);
                    let score = score_map.get(&cp.problem_id).copied().unwrap_or(0);
                    if let Some(&at) = ac_at {
                        total_score += score;
                        last_ac = Some(match last_ac {
                            None => at,
                            Some(prev) => prev.max(at),
                        });
                    }
                    ProblemCell {
                        label: cp.label.clone(),
                        ac_time: ac_at.map(|t| {
                            let secs = (*t - contest.start_time).num_seconds().max(0);
                            fmt_elapsed(secs)
                        }),
                        score: if ac_at.is_some() { score } else { 0 },
                    }
                })
                .collect();

            let elapsed_from_start = last_ac.map(|t| {
                let secs = (t - contest.start_time).num_seconds().max(0);
                fmt_elapsed(secs)
            });

            RowWithRaw {
                row: StandingRow {
                    rank: 0,
                    username: ud.username.clone(),
                    total_score,
                    elapsed_from_start,
                    problems,
                },
                last_ac_raw: last_ac,
            }
        })
        .collect();

    // 順位付け: 得点 DESC, last_ac_raw ASC（None は最後）
    rows_raw.sort_by(|a, b| {
        b.row.total_score.cmp(&a.row.total_score).then_with(|| {
            match (&a.last_ac_raw, &b.last_ac_raw) {
                (Some(at), Some(bt)) => at.cmp(bt),
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, None) => std::cmp::Ordering::Equal,
            }
        })
    });

    let mut rows: Vec<StandingRow> = rows_raw.into_iter().map(|r| r.row).collect();

    let mut prev_score = u64::MAX;
    let mut prev_elapsed: Option<String> = None;
    let mut current_rank = 0usize;
    for (i, row) in rows.iter_mut().enumerate() {
        if row.total_score != prev_score || row.elapsed_from_start != prev_elapsed {
            current_rank = i + 1;
            prev_score = row.total_score;
            prev_elapsed = row.elapsed_from_start.clone();
        }
        row.rank = current_rank;
    }

    #[derive(Serialize)]
    struct ProblemHeader {
        label: String,
        problem_id: String,
        title: String,
        score: u64,
    }

    let problem_headers: Vec<ProblemHeader> = cp_list
        .iter()
        .filter_map(|cp| {
            problem::load_one(&state.problems_dir, &cp.problem_id).ok().map(|p| ProblemHeader {
                label: cp.label.clone(),
                problem_id: cp.problem_id.clone(),
                title: p.title,
                score: p.score,
            })
        })
        .collect();

    let mut ctx = Context::new();
    ctx.insert("contest_id", &contest_id);
    ctx.insert("contest_title", &contest.title);
    ctx.insert("problem_headers", &problem_headers);
    ctx.insert("standings", &rows);
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    render(&state.tera, "contests/standings.html", ctx)
}

// ---- 旧 HTML ハンドラ（/problems, /submissions） ----

pub async fn problems_index(
    State(state): State<AppState>,
    session: Session,
) -> Result<Html<String>, HtmlError> {
    let problems = problem::load_all(&state.problems_dir);
    let mut ctx = Context::new();
    ctx.insert("problems", &problems);
    ctx.insert("contest_id", &Option::<String>::None);
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
    ctx.insert("contest_id", &Option::<String>::None);
    ctx.insert("current_user", &current_username(&session, &state.pool).await);
    ctx.insert("cooldown_remaining_ms", &Option::<i64>::None);
    render(&state.tera, "problems/detail.html", ctx)
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

    let user_id: Option<Uuid> = session.get("user_id").await.ok().flatten();

    let id = Uuid::new_v4();
    let sub = Submission {
        id,
        user_id,
        contest_id: None,
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
                problem_label: String::new(),
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
    ctx.insert("contest_id", &Option::<String>::None);
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
    ctx.insert("contest_id", &Option::<String>::None);
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
        contest_id: None,
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
