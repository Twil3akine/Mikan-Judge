use std::sync::Arc;

use axum::{
    routing::{get, post},
    Router,
};
use sqlx::PgPool;
use tokio::sync::mpsc::Sender;

use crate::worker::JudgeJob;

pub mod handlers;

#[derive(Clone)]
pub struct AppState {
    pub pool: Arc<PgPool>,
    pub job_tx: Sender<JudgeJob>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/submit", post(handlers::submit))
        .route("/result/{id}", get(handlers::get_result))
        .with_state(state)
}
