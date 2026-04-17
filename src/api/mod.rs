use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use sqlx::PgPool;
use tera::Tera;
use tokio::sync::mpsc::Sender;
use tower_http::services::ServeDir;

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
    Router::new()
        // ---- HTML ----
        .route("/", get(handlers::index))
        .route("/problems", get(handlers::problems_index))
        .route("/problems/{id}", get(handlers::problems_detail))
        .route("/problems/{id}/submit", post(handlers::problems_submit))
        .route("/submissions/{id}", get(handlers::submissions_detail))
        .route("/submissions/{id}/poll", get(handlers::submissions_poll))
        // ---- JSON API ----
        .route("/health", get(handlers::health))
        .route("/submit", post(handlers::api_submit))
        .route("/result/{id}", get(handlers::api_get_result))
        // ---- Static files ----
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state)
}
