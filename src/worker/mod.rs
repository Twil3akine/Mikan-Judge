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
    /// (stdin, expected_output) のペアのリスト
    pub testcases: Vec<(String, String)>,
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
        if let Err(e) =
            db_sub::update_result(pool, job.id, &status, None, None, None, Some(msg), None).await
        {
            tracing::error!(%job.id, "failed to update compile error: {e}");
        }
        return;
    }

    // --- 全テストケースを実行 ---
    let mem = job.memory_limit_kb * 1024;
    let cfg = SandboxConfig {
        time_limit: Duration::from_millis(job.time_limit_ms),
        max_output_bytes: 16 * 1024 * 1024,
        // インタプリタ言語は仮想メモリ制限なし（インタプリタ自体が大量の VA を使うため）
        vm_limit_bytes: if job.language.is_interpreted() { None } else { Some(mem * 2) },
    };

    let mut final_status = JudgeStatus::Accepted;
    let mut last_time_ms: Option<u64> = None;
    let mut last_memory_kb: Option<u64> = None;
    let mut last_stdout = String::new();
    let mut last_stderr = String::new();
    let mut tc_verdicts: Vec<String> = Vec::new();

    for (stdin, expected_output) in &job.testcases {
        let run = match sandbox::run_in_sandbox(
            &compiled.executable,
            compiled.run_args.clone(),
            stdin.as_bytes(),
            cfg.clone(),
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                set_status(pool, job.id, JudgeStatus::InternalError { message: e.to_string() })
                    .await;
                return;
            }
        };

        last_time_ms = Some(run.cpu_time_used.as_millis() as u64);
        let memory_used_kb = run.memory_used_bytes / 1024;
        last_memory_kb = Some(memory_used_kb);
        last_stdout = String::from_utf8_lossy(&run.stdout).into_owned();
        let runtime_stderr = String::from_utf8_lossy(&run.stderr).into_owned();
        last_stderr = if compiled.warnings.is_empty() {
            runtime_stderr
        } else if runtime_stderr.is_empty() {
            format!("[Compile warnings]\n{}", compiled.warnings)
        } else {
            format!("[Compile warnings]\n{}\n[Runtime stderr]\n{runtime_stderr}", compiled.warnings)
        };

        let case_status = match run.status {
            RunStatus::TimeLimitExceeded => JudgeStatus::TimeLimitExceeded,
            RunStatus::RuntimeError | RunStatus::Killed => {
                // RLIMIT_AS 超過で kill された場合はメモリ使用量で MLE を判定する
                if memory_used_kb > job.memory_limit_kb {
                    JudgeStatus::MemoryLimitExceeded
                } else {
                    JudgeStatus::RuntimeError { exit_code: run.exit_code.unwrap_or(-1) }
                }
            }
            RunStatus::Ok => {
                if String::from_utf8_lossy(&run.stdout).trim() == expected_output.trim() {
                    JudgeStatus::Accepted
                } else {
                    JudgeStatus::WrongAnswer
                }
            }
        };

        tc_verdicts.push(match &case_status {
            JudgeStatus::Accepted => "AC",
            JudgeStatus::WrongAnswer => "WA",
            JudgeStatus::TimeLimitExceeded => "TLE",
            JudgeStatus::MemoryLimitExceeded => "MLE",
            JudgeStatus::RuntimeError { .. } => "RE",
            _ => "IE",
        }.to_string());
        if !matches!(case_status, JudgeStatus::Accepted) {
            // 残りのケースは未実施 (skipped) として記録
            let remaining = job.testcases.len() - tc_verdicts.len();
            tc_verdicts.extend(std::iter::repeat_n("--".to_string(), remaining));
            final_status = case_status;
            break;
        }
    }

    if let Err(e) = db_sub::update_result(
        pool,
        job.id,
        &final_status,
        last_time_ms,
        last_memory_kb,
        Some(&last_stdout),
        Some(&last_stderr),
        Some(&tc_verdicts),
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
