use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use sqlx::PgPool;
use tera::Tera;
use tokio::sync::mpsc::Sender;
use tower_http::services::ServeDir;
use tower_sessions::{MemoryStore, SessionManagerLayer, cookie::SameSite};

use crate::worker::JudgeJob;

pub mod handlers;

#[derive(Clone)]
pub struct AppState {
    pub pool: Arc<PgPool>,
    pub job_tx: Sender<JudgeJob>,
    pub tera: Arc<Tera>,
    pub problems_dir: Arc<std::path::PathBuf>,
}

pub fn create_router(state: AppState) -> Router {
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_same_site(SameSite::Lax);

    Router::new()
        // ---- 認証 ----
        .route("/register", get(handlers::register_form).post(handlers::register))
        .route("/login",    get(handlers::login_form).post(handlers::login))
        .route("/logout",   post(handlers::logout))
        // ---- HTML ----
        .route("/", get(handlers::index))
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
