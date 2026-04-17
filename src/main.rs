use std::sync::Arc;

mod api;
mod db;
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

    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set (run inside nix dev shell)");

    let pool = Arc::new(
        db::create_pool(&database_url)
            .await
            .expect("Failed to connect to database"),
    );

    let num_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2);
    tracing::info!("Starting {num_workers} judge worker(s)");

    let job_tx = worker::spawn_workers(num_workers, pool.clone());

    let state = api::AppState { pool, job_tx };
    let app = api::create_router(state);

    let addr = "0.0.0.0:3000";
    tracing::info!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
