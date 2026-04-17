use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Mutex, RwLock};
use uuid::Uuid;

use crate::sandbox::{self, RunStatus, SandboxConfig};
use crate::types::{JudgeStatus, Language, Submission};

/// 提出情報をインメモリで保持するストア（後で PostgreSQL に差し替える）
pub type SubmissionStore = Arc<RwLock<HashMap<Uuid, Submission>>>;

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
/// 返り値の `Sender` にジョブを送ると、空いているワーカーが処理する。
pub fn spawn_workers(num_workers: usize, store: SubmissionStore) -> mpsc::Sender<JudgeJob> {
    let (tx, rx) = mpsc::channel::<JudgeJob>(256);
    // Arc<Mutex<Receiver>> で複数ワーカーが単一のチャネルを共有する
    let rx = Arc::new(Mutex::new(rx));

    for worker_id in 0..num_workers {
        let rx = rx.clone();
        let store = store.clone();
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
                        judge(job, store.clone()).await;
                    }
                }
            }
        });
    }

    tx
}

// ---- 内部実装 ----

async fn set_status(store: &SubmissionStore, id: Uuid, status: JudgeStatus) {
    if let Some(sub) = store.write().await.get_mut(&id) {
        sub.status = status;
    }
}

async fn judge(job: JudgeJob, store: SubmissionStore) {
    set_status(&store, job.id, JudgeStatus::Running).await;

    // 一時ディレクトリ（ソースとバイナリを置く）
    let work_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => {
            set_status(
                &store,
                job.id,
                JudgeStatus::InternalError { message: e.to_string() },
            )
            .await;
            return;
        }
    };

    // --- コンパイル ---
    let (exe, compile_err) =
        match sandbox::compile(&job.source_code, &job.language, work_dir.path()).await {
            Ok(r) => r,
            Err(e) => {
                set_status(
                    &store,
                    job.id,
                    JudgeStatus::InternalError { message: e.to_string() },
                )
                .await;
                return;
            }
        };

    if let Some(msg) = compile_err {
        set_status(&store, job.id, JudgeStatus::CompileError { message: msg }).await;
        return;
    }

    // --- サンドボックス実行 ---
    let cfg = SandboxConfig {
        time_limit: Duration::from_millis(job.time_limit_ms),
        memory_limit_bytes: job.memory_limit_kb * 1024,
        max_output_bytes: 16 * 1024 * 1024, // 16 MiB
    };

    let run = match sandbox::run_in_sandbox(&exe, job.stdin.as_bytes(), cfg).await {
        Ok(r) => r,
        Err(e) => {
            set_status(
                &store,
                job.id,
                JudgeStatus::InternalError { message: e.to_string() },
            )
            .await;
            return;
        }
    };

    // --- 判定 ---
    let final_status = match run.status {
        RunStatus::TimeLimitExceeded => JudgeStatus::TimeLimitExceeded,
        RunStatus::MemoryLimitExceeded => JudgeStatus::MemoryLimitExceeded,
        RunStatus::RuntimeError | RunStatus::Killed(_) => JudgeStatus::RuntimeError {
            exit_code: run.exit_code.unwrap_or(-1),
        },
        RunStatus::Ok => {
            // 末尾の空白・改行を無視して比較
            if String::from_utf8_lossy(&run.stdout).trim() == job.expected_output.trim() {
                JudgeStatus::Accepted
            } else {
                JudgeStatus::WrongAnswer
            }
        }
    };

    // --- 結果をストアに書き戻す ---
    let mut map = store.write().await;
    if let Some(sub) = map.get_mut(&job.id) {
        sub.status = final_status;
        sub.time_used_ms = Some(run.time_used.as_millis() as u64);
        sub.memory_used_kb = Some(run.memory_used_bytes / 1024);
        sub.stdout = Some(String::from_utf8_lossy(&run.stdout).into_owned());
        sub.stderr = Some(String::from_utf8_lossy(&run.stderr).into_owned());
    }
}
