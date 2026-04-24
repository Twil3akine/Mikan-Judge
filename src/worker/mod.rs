use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::db::submission as db_sub;
use crate::sandbox::{self, RunStatus, SandboxConfig};
use crate::types::{JudgeStatus, JudgeType, Language, Submission, TestcaseVerdict};

/// ジャッジキューに積むジョブ
#[derive(Debug)]
pub struct JudgeJob {
    pub id: Uuid,
    pub source_code: String,
    pub language: Language,
    /// (stdin, expected_output) のペアのリスト。expected は exact のみ Some
    pub testcases: Vec<(String, Option<String>)>,
    pub time_limit_ms: u64,
    pub memory_limit_kb: u64,
    pub judge_type: JudgeType,
    /// ヒューリスティック: 問題ディレクトリ内の scorer.py パス
    pub scorer_path: Option<PathBuf>,
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
            set_status(
                pool,
                job.id,
                JudgeStatus::InternalError {
                    message: e.to_string(),
                },
            )
            .await;
            return;
        }
    };

    // --- コンパイル ---
    let compiled = match sandbox::compile(&job.source_code, &job.language, work_dir.path()).await {
        Ok(r) => r,
        Err(e) => {
            set_status(
                pool,
                job.id,
                JudgeStatus::InternalError {
                    message: e.to_string(),
                },
            )
            .await;
            return;
        }
    };

    if let Some(ref msg) = compiled.error {
        let status = JudgeStatus::CompileError {
            message: msg.clone(),
        };
        if let Err(e) = db_sub::update_result(
            pool,
            job.id,
            &status,
            None,
            None,
            None,
            Some(msg),
            None,
            None,
        )
        .await
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
        vm_limit_bytes: if job.language.is_interpreted() {
            None
        } else {
            Some(mem * 2)
        },
    };

    let is_heuristic = job.judge_type == JudgeType::Heuristic;
    let mut final_status = if is_heuristic {
        JudgeStatus::Scored
    } else {
        JudgeStatus::Accepted
    };
    let mut max_time_ms: u64 = 0;
    let mut max_memory_kb: u64 = 0;
    let mut first_fail_stdout = String::new();
    let mut first_fail_stderr = String::new();
    let mut tc_verdicts: Vec<TestcaseVerdict> = Vec::new();
    let mut total_score: f64 = 0.0;

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
                set_status(
                    pool,
                    job.id,
                    JudgeStatus::InternalError {
                        message: e.to_string(),
                    },
                )
                .await;
                return;
            }
        };

        let time_ms = run.wall_time_used.as_millis() as u64;
        let memory_used_kb = run.memory_used_bytes / 1024;
        if time_ms > max_time_ms {
            max_time_ms = time_ms;
        }
        if memory_used_kb > max_memory_kb {
            max_memory_kb = memory_used_kb;
        }

        let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
        let runtime_stderr = String::from_utf8_lossy(&run.stderr).into_owned();
        let stderr = if compiled.warnings.is_empty() {
            runtime_stderr
        } else if runtime_stderr.is_empty() {
            format!("[Compile warnings]\n{}", compiled.warnings)
        } else {
            format!(
                "[Compile warnings]\n{}\n[Runtime stderr]\n{runtime_stderr}",
                compiled.warnings
            )
        };

        let (case_status, case_score) = match run.status {
            RunStatus::TimeLimitExceeded => (JudgeStatus::TimeLimitExceeded, None),
            RunStatus::RuntimeError | RunStatus::Killed => {
                // RLIMIT_AS 超過で kill された場合はメモリ使用量で MLE を判定する
                let s = if memory_used_kb > job.memory_limit_kb {
                    JudgeStatus::MemoryLimitExceeded
                } else {
                    JudgeStatus::RuntimeError {
                        exit_code: run.exit_code.unwrap_or(-1),
                    }
                };
                (s, None)
            }
            RunStatus::Ok => {
                if is_heuristic {
                    match run_scorer(job.scorer_path.as_deref(), stdin, &stdout, work_dir.path())
                        .await
                    {
                        Ok(score) => (JudgeStatus::Scored, Some(score)),
                        Err(e) => (
                            JudgeStatus::InternalError {
                                message: e.to_string(),
                            },
                            None,
                        ),
                    }
                } else {
                    let expected = expected_output.as_deref().unwrap_or("");
                    if stdout.trim() == expected.trim() {
                        (JudgeStatus::Accepted, None)
                    } else {
                        (JudgeStatus::WrongAnswer, None)
                    }
                }
            }
        };

        if let Some(s) = case_score {
            total_score += s;
        }

        let verdict_str = match &case_status {
            JudgeStatus::Accepted => "AC",
            JudgeStatus::WrongAnswer => "WA",
            JudgeStatus::TimeLimitExceeded => "TLE",
            JudgeStatus::MemoryLimitExceeded => "MLE",
            JudgeStatus::RuntimeError { .. } => "RE",
            JudgeStatus::Scored => "SCORED",
            _ => "IE",
        };
        tc_verdicts.push(TestcaseVerdict {
            verdict: verdict_str.to_string(),
            time_ms: Some(time_ms),
            memory_kb: Some(memory_used_kb),
            score: case_score,
        });

        // 最初の失敗ケースを最終ステータスに記録し、以降も全ケース実行を継続する
        let is_success = matches!(case_status, JudgeStatus::Accepted | JudgeStatus::Scored);
        let current_ok = matches!(final_status, JudgeStatus::Accepted | JudgeStatus::Scored);
        if current_ok && !is_success {
            final_status = case_status;
            first_fail_stdout = stdout;
            first_fail_stderr = stderr;
        }
    }

    let (last_stdout, last_stderr) =
        if matches!(final_status, JudgeStatus::Accepted | JudgeStatus::Scored) {
            (String::new(), String::new())
        } else {
            (first_fail_stdout, first_fail_stderr)
        };

    let score = if is_heuristic {
        Some(total_score)
    } else {
        None
    };

    if let Err(e) = db_sub::update_result(
        pool,
        job.id,
        &final_status,
        if max_time_ms > 0 {
            Some(max_time_ms)
        } else {
            None
        },
        if max_memory_kb > 0 {
            Some(max_memory_kb)
        } else {
            None
        },
        Some(&last_stdout),
        Some(&last_stderr),
        Some(&tc_verdicts),
        score,
    )
    .await
    {
        tracing::error!(%job.id, "failed to write result: {e}");
    }
}

/// ヒューリスティック用スコアラー実行。
/// `python3 scorer.py <input_file> <output_file>` を呼び出し、stdout を f64 として返す。
async fn run_scorer(
    scorer_path: Option<&Path>,
    input: &str,
    output: &str,
    work_dir: &Path,
) -> anyhow::Result<f64> {
    let scorer = scorer_path.ok_or_else(|| {
        anyhow::anyhow!("scorer.py が見つかりません（judge_type=heuristic にも関わらず）")
    })?;

    let in_file = work_dir.join("scorer_input.txt");
    let out_file = work_dir.join("scorer_output.txt");
    std::fs::write(&in_file, input)?;
    std::fs::write(&out_file, output)?;

    let result = tokio::process::Command::new("python3")
        .arg(scorer)
        .arg(&in_file)
        .arg(&out_file)
        .output()
        .await?;

    if !result.status.success() {
        let msg = String::from_utf8_lossy(&result.stderr);
        anyhow::bail!("スコアラー実行失敗: {msg}");
    }

    let score_str = String::from_utf8_lossy(&result.stdout).trim().to_string();
    score_str
        .parse::<f64>()
        .map_err(|e| anyhow::anyhow!("スコアのパース失敗 ({score_str:?}): {e}"))
}

/// API ハンドラが提出を登録するときに使う
pub async fn create_submission(pool: &PgPool, sub: &Submission) -> anyhow::Result<()> {
    db_sub::insert(pool, sub).await
}
