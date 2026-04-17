use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::db::submission as db_sub;
use crate::sandbox::{self, RunStatus, SandboxConfig};
use crate::types::{JudgeStatus, Language, Submission};

/// ジャッジキューに積むジョブ
#[derive(Debug)]
pub struct JudgeJob {
    pub id: Uuid,
    pub source_code: String,
    pub language: Language,
    pub stdin: String,
    pub expected_output: String,
    pub time_limit_ms: u64,
    pub memory_limit_kb: u64,
}

/// `num_workers` 個の tokio タスクを起動してジョブを並列処理する。
pub fn spawn_workers(num_workers: usize, pool: Arc<PgPool>) -> mpsc::Sender<JudgeJob> {
    let (tx, rx) = mpsc::channel::<JudgeJob>(256);
    let rx = Arc::new(tokio::sync::Mutex::new(rx));

    for worker_id in 0..num_workers {
        let rx = rx.clone();
        let pool = pool.clone();
        tokio::spawn(async move {
            tracing::info!(worker_id, "judge worker started");
            loop {
                let job = rx.lock().await.recv().await;
                match job {
                    None => {
                        tracing::info!(worker_id, "channel closed, worker shutting down");
                        break;
                    }
                    Some(job) => {
                        tracing::info!(worker_id, submission_id = %job.id, "processing job");
                        judge(job, &pool).await;
                    }
                }
            }
        });
    }

    tx
}

// ---- 内部実装 ----

async fn set_status(pool: &PgPool, id: Uuid, status: JudgeStatus) {
    if let Err(e) = db_sub::update_status(pool, id, &status).await {
        tracing::error!(%id, "failed to update status: {e}");
    }
}

async fn judge(job: JudgeJob, pool: &PgPool) {
    set_status(pool, job.id, JudgeStatus::Running).await;

    let work_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            set_status(pool, job.id, JudgeStatus::InternalError { message: e.to_string() }).await;
            return;
        }
    };

    // --- コンパイル ---
    let compiled =
        match sandbox::compile(&job.source_code, &job.language, work_dir.path()).await {
            Ok(r) => r,
            Err(e) => {
                set_status(pool, job.id, JudgeStatus::InternalError { message: e.to_string() })
                    .await;
                return;
            }
        };

    if let Some(ref msg) = compiled.error {
        let status = JudgeStatus::CompileError { message: msg.clone() };
        if let Err(e) = db_sub::update_result(pool, job.id, &status, None, None, None, Some(msg)).await
        {
            tracing::error!(%job.id, "failed to update compile error: {e}");
        }
        return;
    }

    // --- サンドボックス実行 ---
    let mem = job.memory_limit_kb * 1024;
    let cfg = SandboxConfig {
        time_limit: Duration::from_millis(job.time_limit_ms),
        max_output_bytes: 16 * 1024 * 1024,
        // インタプリタ言語は仮想メモリ制限なし（インタプリタ自体が大量の VA を使うため）
        vm_limit_bytes: if job.language.is_interpreted() { None } else { Some(mem * 2) },
    };

    let run = match sandbox::run_in_sandbox(&compiled.executable, compiled.run_args, job.stdin.as_bytes(), cfg).await {
        Ok(r) => r,
        Err(e) => {
            set_status(pool, job.id, JudgeStatus::InternalError { message: e.to_string() }).await;
            return;
        }
    };

    // --- 判定 ---
    let final_status = match run.status {
        RunStatus::TimeLimitExceeded => JudgeStatus::TimeLimitExceeded,
        RunStatus::MemoryLimitExceeded => JudgeStatus::MemoryLimitExceeded,
        RunStatus::RuntimeError | RunStatus::Killed(_) => {
            JudgeStatus::RuntimeError { exit_code: run.exit_code.unwrap_or(-1) }
        }
        RunStatus::Ok => {
            if String::from_utf8_lossy(&run.stdout).trim() == job.expected_output.trim() {
                JudgeStatus::Accepted
            } else {
                JudgeStatus::WrongAnswer
            }
        }
    };

    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let runtime_stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    // コンパイル警告があれば先頭に表示
    let stderr = if compiled.warnings.is_empty() {
        runtime_stderr
    } else if runtime_stderr.is_empty() {
        format!("[Compile warnings]\n{}", compiled.warnings)
    } else {
        format!("[Compile warnings]\n{}\n[Runtime stderr]\n{runtime_stderr}", compiled.warnings)
    };

    if let Err(e) = db_sub::update_result(
        pool,
        job.id,
        &final_status,
        Some(run.time_used.as_millis() as u64),
        Some(run.memory_used_bytes / 1024),
        Some(&stdout),
        Some(&stderr),
    )
    .await
    {
        tracing::error!(%job.id, "failed to write result: {e}");
    }
}

/// API ハンドラが提出を登録するときに使う
pub async fn create_submission(pool: &PgPool, sub: &Submission) -> anyhow::Result<()> {
    db_sub::insert(pool, sub).await
}
