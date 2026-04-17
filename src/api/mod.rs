use axum::{
    routing::{get, post},
    Router,
};
use tokio::sync::mpsc::Sender;

use crate::worker::{JudgeJob, SubmissionStore};

pub mod handlers;

/// axum の共有ステート
#[derive(Clone)]
pub struct AppState {
    pub store: SubmissionStore,
    pub job_tx: Sender<JudgeJob>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/submit", post(handlers::submit))
        .route("/result/{id}", get(handlers::get_result))
        .with_state(state)
}
