use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

mod api;
mod sandbox;
mod types;
mod worker;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("mikan_judge=info".parse().unwrap()),
        )
        .init();

    let store: worker::SubmissionStore = Arc::new(RwLock::new(HashMap::new()));

    let num_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    tracing::info!("Starting {num_workers} judge worker(s)");

    let job_tx = worker::spawn_workers(num_workers, store.clone());

    let state = api::AppState { store, job_tx };
    let app = api::create_router(state);

    let addr = "0.0.0.0:3000";
    tracing::info!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
