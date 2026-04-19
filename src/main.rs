use std::sync::Arc;

mod api;
mod db;
mod problem;
mod sandbox;
mod session_store;
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

    let tera = Arc::new(
        tera::Tera::new("templates/**/*.html").expect("Failed to load templates"),
    );
    let problems_dir = Arc::new(std::path::PathBuf::from("problems"));

    let lang_versions = Arc::new(types::LanguageVersions::detect().await);
    tracing::info!(
        cpp = %lang_versions.cpp, rust = %lang_versions.rust,
        python = %lang_versions.python, pypy = %lang_versions.pypy,
        "detected language versions"
    );

    let state = api::AppState { pool, job_tx, tera, problems_dir, lang_versions };
    let app = api::create_router(state).await;

    let addr = "0.0.0.0:3000";
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to bind to {addr}: {e} (port already in use?)");
            std::process::exit(1);
        }
    };
    tracing::info!("Listening on {addr}");
    axum::serve(listener, app).await.unwrap();
}
