use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use sqlx::PgPool;
use tera::Tera;
use tokio::sync::mpsc::Sender;
use tower_http::services::ServeDir;
use tower_sessions::{SessionManagerLayer, cookie::SameSite};

use crate::session_store::PgSessionStore;
use crate::types::LanguageVersions;
use crate::worker::JudgeJob;

pub mod handlers;

#[derive(Clone)]
pub struct AppState {
    pub pool: Arc<PgPool>,
    pub job_tx: Sender<JudgeJob>,
    pub tera: Arc<Tera>,
    pub problems_dir: Arc<std::path::PathBuf>,
    pub lang_versions: Arc<LanguageVersions>,
}

pub async fn create_router(state: AppState) -> Router {
    // PostgreSQL-backed session store — sessions survive server restarts
    // tower_sessions テーブルは migrations/004_create_sessions.sql で作成済み
    let session_store = PgSessionStore::new((*state.pool).clone());
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_same_site(SameSite::Lax);

    Router::new()
        // ---- 認証 ----
        .route("/register", get(handlers::register_form).post(handlers::register))
        .route("/login",    get(handlers::login_form).post(handlers::login))
        .route("/logout",   post(handlers::logout))
        // ---- ユーザー設定 ----
        .route("/settings", get(handlers::settings_get).post(handlers::settings_post))
        .route("/settings/delete-account", post(handlers::delete_account))
        // ---- トップ（ランディング）・コンテスト一覧 ----
        .route("/", get(handlers::index))
        .route("/languages", get(handlers::languages))
        .route("/contests", get(handlers::contests_index))
        // ---- コンテスト内ルート ----
        .route("/contests/{contest_id}", get(handlers::contest_detail))
        .route("/contests/{contest_id}/problems", get(handlers::contest_problems_index))
        .route("/contests/{contest_id}/problems/{problem_id}", get(handlers::contest_problem_detail))
        .route("/contests/{contest_id}/problems/{problem_id}/submit", post(handlers::contest_problem_submit))
        .route("/contests/{contest_id}/submissions", get(handlers::contest_submissions_index))
        .route("/contests/{contest_id}/submissions/{id}", get(handlers::contest_submission_detail))
        .route("/contests/{contest_id}/submissions/{id}/poll", get(handlers::contest_submission_poll))
        .route("/contests/{contest_id}/standings", get(handlers::contest_standings))
        // ---- 旧 HTML（後方互換） ----
        .route("/problems", get(handlers::problems_index))
        .route("/problems/{id}", get(handlers::problems_detail))
        .route("/problems/{id}/submit", post(handlers::problems_submit))
        .route("/submissions", get(handlers::submissions_index))
        .route("/submissions/{id}", get(handlers::submissions_detail))
        .route("/submissions/{id}/poll", get(handlers::submissions_poll))
        // ---- JSON API ----
        .route("/health", get(handlers::health))
        .route("/submit", post(handlers::api_submit))
        .route("/result/{id}", get(handlers::api_get_result))
        // ---- Static files ----
        .nest_service("/static", ServeDir::new("static"))
        .layer(session_layer)
        .with_state(state)
}
