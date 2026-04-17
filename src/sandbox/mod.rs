use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use tokio::process::Command;
use tokio::time::timeout;

use crate::types::Language;

pub mod runner;
pub mod seccomp;

/// サンドボックス実行の設定
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub time_limit: Duration,
    pub memory_limit_bytes: u64,
    /// 標準出力の最大バイト数（超えたら切り捨て）
    pub max_output_bytes: usize,
}

/// サンドボックス内実行の結果
#[derive(Debug)]
pub struct RunResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
    pub time_used: Duration,
    /// getrusage(RUSAGE_CHILDREN).max_rss から取得（バイト単位）
    pub memory_used_bytes: u64,
    pub status: RunStatus,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum RunStatus {
    Ok,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    RuntimeError,
    /// シグナルで強制終了（シグナル番号）
    Killed(i32),
}

/// ソースコードをコンパイルして実行バイナリを生成する。
///
/// 成功: `(実行ファイルのパス, None)`
/// コンパイルエラー: `(_, Some(エラーメッセージ))`
pub async fn compile(
    source_code: &str,
    language: &Language,
    work_dir: &Path,
) -> Result<(PathBuf, Option<String>)> {
    let src_path = work_dir.join(format!("solution.{}", language.extension()));
    tokio::fs::write(&src_path, source_code).await?;

    let output_path = work_dir.join("solution");
    let args = language.compile_args(
        src_path.to_str().unwrap(),
        output_path.to_str().unwrap(),
    );

    let result = timeout(
        Duration::from_secs(30),
        Command::new(language.compiler()).args(&args).output(),
    )
    .await;

    match result {
        Err(_elapsed) => anyhow::bail!("コンパイルが30秒でタイムアウトしました"),
        Ok(Err(e)) => anyhow::bail!("コンパイラの起動に失敗しました: {e}"),
        Ok(Ok(out)) => {
            if out.status.success() {
                Ok((output_path, None))
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                Ok((output_path, Some(stderr)))
            }
        }
    }
}

/// 実行ファイルをサンドボックス内で実行して結果を返す。
///
/// 内部で `spawn_blocking` → `fork` を使うため、tokio ランタイム上でも安全に呼べる。
pub async fn run_in_sandbox(
    executable: &Path,
    stdin: &[u8],
    config: SandboxConfig,
) -> Result<RunResult> {
    let executable = executable.to_owned();
    let stdin = stdin.to_owned();

    tokio::task::spawn_blocking(move || {
        runner::run_sandboxed_blocking(&executable, &stdin, &config)
    })
    .await?
}
